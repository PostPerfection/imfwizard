//! What the picture flags do to the pixels the IMP ships.
//!
//! Every test builds a source whose expected output is exact, runs the real
//! `imfwizard create`, then opens the picture MXF it wrote and decodes a frame,
//! so nothing here can pass on the plan arithmetic alone.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FPS: u32 = 24;

/// The largest 12-bit sample, what a full-scale component decodes back to.
const FULL_SCALE: i32 = 4095;

/// A component that carried 255 has to come back near full scale, and one that
/// carried 0 has to stay near zero. The encode runs at a 10:1 ratio, which is
/// near lossless on the flat colours these sources are made of.
const HIGH: i32 = 3600;
const LOW: i32 = 400;

/// Half scale, which separates the white marker from the black around it even
/// where the fit scaled its edge.
const MARKER_THRESHOLD: i32 = FULL_SCALE / 2;

/// Frames each source holds. Two is enough for every measurement but the
/// denoise, whose temporal pass needs history.
const FRAMES: u32 = 2;
const DENOISE_FRAMES: u32 = 4;

/// Width of the coloured band drawn down each edge of the crop source.
const BAND: u32 = 100;

/// Height of the black bar the auto-crop source is padded with, top and bottom.
const BAR: u32 = 140;
/// What is left of the auto-crop source once both bars are cropped away.
const ACTIVE_HEIGHT: u32 = HEIGHT - 2 * BAR;

/// DCI 2K full container, the raster the auto-crop and fill tests land on.
const RASTER_2K: &str = "2048x1080";
const RASTER_2K_WIDTH: u32 = 2048;
const RASTER_2K_HEIGHT: u32 = 1080;

/// cropdetect rounds to even and the fit rounds the scale to even, so a
/// measured edge may miss the arithmetic by a few rows.
const FIT_TOLERANCE: u32 = 6;
/// A flip neither scales nor pads, so its marker lands exactly.
const EXACT_TOLERANCE: u32 = 2;

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

/// Build a clip from a lavfi source and a filter chain, losslessly, so the
/// samples that reach the encoder are the ones the filters produced.
fn build_clip(path: &Path, source: &str, filters: &str, frames: u32, pixel_format: &str) {
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", source])
        .args(["-vf", filters])
        .args(["-frames:v", &frames.to_string()])
        .args(["-c:v", "ffv1", "-pix_fmt", pixel_format])
        .arg(path)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
}

/// Run `create` over a clip with the picture flags under test.
fn create(dir: &Path, name: &str, clip: &Path, picture_flags: &[&str]) -> PathBuf {
    let imp = dir.join(name);
    cmd()
        .args([
            "create",
            "-o",
            &imp.to_string_lossy(),
            "-t",
            "Picture Processing",
            "--video",
            &clip.to_string_lossy(),
            "--fps-num",
            &FPS.to_string(),
            "--fps-den",
            "1",
        ])
        .args(picture_flags)
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP created"));
    imp
}

/// A decoded frame out of the picture track file an IMP holds.
struct Frame {
    width: u32,
    height: u32,
    components: Vec<Vec<i32>>,
}

impl Frame {
    fn pixel(&self, x: u32, y: u32) -> (i32, i32, i32) {
        let index = (y * self.width + x) as usize;
        (
            self.components[0][index],
            self.components[1][index],
            self.components[2][index],
        )
    }

    /// Mean absolute difference between vertically adjacent pixels, in 12-bit
    /// codes. Combing puts every second row out of step with its neighbours, so
    /// this is what a deinterlace has to bring down.
    fn row_to_row_difference(&self) -> f64 {
        let green = &self.components[1];
        let mut total = 0i64;
        for row in 0..self.height - 1 {
            for column in 0..self.width {
                let here = green[(row * self.width + column) as usize];
                let below = green[((row + 1) * self.width + column) as usize];
                total += (here - below).abs() as i64;
            }
        }
        total as f64 / ((self.height - 1) * self.width) as f64
    }

    /// Mean absolute difference between horizontally adjacent pixels, in 12-bit
    /// codes. On a flat source this is all noise.
    fn pixel_to_pixel_difference(&self) -> f64 {
        let green = &self.components[1];
        let mut total = 0i64;
        for row in 0..self.height {
            for column in 0..self.width - 1 {
                let here = green[(row * self.width + column) as usize];
                let right = green[(row * self.width + column + 1) as usize];
                total += (here - right).abs() as i64;
            }
        }
        total as f64 / (self.height * (self.width - 1)) as f64
    }

    /// The box around every pixel brighter than `threshold`, as
    /// (left, right, top, bottom) with right and bottom inclusive.
    fn bright_bounds(&self, threshold: i32) -> (u32, u32, u32, u32) {
        let (mut left, mut right, mut top, mut bottom) = (u32::MAX, 0u32, u32::MAX, 0u32);
        for row in 0..self.height {
            for column in 0..self.width {
                let index = (row * self.width + column) as usize;
                if self.components[1][index] <= threshold {
                    continue;
                }
                left = left.min(column);
                right = right.max(column);
                top = top.min(row);
                bottom = bottom.max(row);
            }
        }
        assert!(left != u32::MAX, "no pixel is brighter than {threshold}");
        (left, right, top, bottom)
    }

    /// The first and last row holding picture on the centre column, and how many
    /// rows that spans.
    fn content_rows(&self) -> (u32, u32, u32) {
        let column = self.width / 2;
        let lit = |row: u32| self.components[1][(row * self.width + column) as usize] > HIGH;
        let first = (0..self.height).find(|row| lit(*row)).expect("no content");
        let last = (0..self.height).rev().find(|row| lit(*row)).unwrap();
        (first, last, last - first + 1)
    }
}

/// The one picture track file in a written IMP.
fn picture_track_file(imp: &Path) -> PathBuf {
    let mut pictures: Vec<PathBuf> = std::fs::read_dir(imp)
        .expect("the IMP directory")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(imfwizard_core::imp::PICTURE_PREFIX))
        })
        .collect();
    assert_eq!(pictures.len(), 1, "expected one picture track file");
    pictures.pop().unwrap()
}

/// Codestream buffer for one frame. The encode runs at a 10:1 ratio, so a 2K
/// frame lands two orders of magnitude under this.
const FRAME_BUFFER_BYTES: usize = 16 << 20;

/// Read one frame out of the packaged essence and decode it.
fn packaged_frame(imp: &Path, index: u32) -> Frame {
    let picture = picture_track_file(imp);
    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    reader
        .open_read(&picture.to_string_lossy())
        .expect("the picture MXF opens");
    let mut buffer = vec![0u8; FRAME_BUFFER_BYTES];
    let size = reader.read_frame(index, &mut buffer, None, None).unwrap();
    buffer.truncate(size);
    reader.close().unwrap();

    let decoded =
        postkit::grok_decoder::decode(buffer, 0).expect("the wrapped frame has to decode");
    assert_eq!(decoded.precision, 12);
    Frame {
        width: decoded.width,
        height: decoded.height,
        components: decoded.components,
    }
}

/// A blue frame with a coloured band down each edge. Each band owns a component
/// the interior has none of, so a band that survived a crop shows up as red or
/// green where the picture should be black.
fn banded_clip(path: &Path) {
    build_clip(
        path,
        &format!("color=c=blue:s={WIDTH}x{HEIGHT}:r={FPS}"),
        &format!(
            "format=gbrp,\
             drawbox=x=0:y=0:w={BAND}:h={HEIGHT}:color=red:t=fill,\
             drawbox=x={}:y=0:w={BAND}:h={HEIGHT}:color=green:t=fill,\
             drawbox=x=0:y=0:w={WIDTH}:h={BAND}:color=yellow:t=fill,\
             drawbox=x=0:y={}:w={WIDTH}:h={BAND}:color=magenta:t=fill",
            WIDTH - BAND,
            HEIGHT - BAND
        ),
        FRAMES,
        "gbrp",
    );
}

/// A white frame with a black bar above and below the picture.
fn barred_clip(path: &Path) {
    build_clip(
        path,
        &format!("color=c=white:s={WIDTH}x{ACTIVE_HEIGHT}:r={FPS}"),
        &format!("format=gbrp,pad={WIDTH}:{HEIGHT}:0:{BAR}:black"),
        FRAMES,
        "gbrp",
    );
}

/// A black frame with a white square in the top-left corner.
fn marked_clip(path: &Path, marker: u32) {
    build_clip(
        path,
        &format!("color=c=black:s={WIDTH}x{HEIGHT}:r={FPS}"),
        &format!("format=gbrp,drawbox=x=0:y=0:w={marker}:h={marker}:color=white:t=fill"),
        FRAMES,
        "gbrp",
    );
}

#[test]
fn a_per_side_crop_removes_that_side_and_leaves_the_others() {
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("banded.mkv");
    banded_clip(&clip);

    // 1920 wide less 200 fits the 1920x1080 raster untouched, so the remaining
    // picture is pillarboxed by exactly what was cut
    let sides = create(
        dir.path(),
        "sides",
        &clip,
        &[
            "--crop-left",
            &BAND.to_string(),
            "--crop-right",
            &BAND.to_string(),
        ],
    );
    let frame = packaged_frame(&sides, 0);
    assert_eq!((frame.width, frame.height), (WIDTH, HEIGHT));

    let middle = HEIGHT / 2;
    let (left_red, _, _) = frame.pixel(BAND / 2, middle);
    let (_, right_green, _) = frame.pixel(WIDTH - BAND / 2, middle);
    assert!(
        left_red < LOW,
        "the red left band survived --crop-left: red {left_red} at x {}",
        BAND / 2
    );
    assert!(
        right_green < LOW,
        "the green right band survived --crop-right: green {right_green} at x {}",
        WIDTH - BAND / 2
    );

    let (red, green, blue) = frame.pixel(WIDTH / 2, middle);
    assert!(
        blue > HIGH && red < LOW && green < LOW,
        "the blue interior is not on the middle row: {red},{green},{blue}"
    );
    let (top_red, top_green, _) = frame.pixel(WIDTH / 2, BAND / 2);
    assert!(
        top_red > HIGH && top_green > HIGH,
        "the yellow top band was cut without --crop-top: {top_red},{top_green}"
    );

    // 1080 tall less 200 fits the same raster untouched, letterboxed by what
    // was cut
    let ends = create(
        dir.path(),
        "ends",
        &clip,
        &[
            "--crop-top",
            &BAND.to_string(),
            "--crop-bottom",
            &BAND.to_string(),
        ],
    );
    let frame = packaged_frame(&ends, 0);
    assert_eq!((frame.width, frame.height), (WIDTH, HEIGHT));

    let (top_red, top_green, _) = frame.pixel(WIDTH / 2, BAND / 2);
    assert!(
        top_red < LOW && top_green < LOW,
        "the yellow top band survived --crop-top: {top_red},{top_green}"
    );
    let (bottom_red, _, _) = frame.pixel(WIDTH / 2, HEIGHT - BAND / 2);
    assert!(
        bottom_red < LOW,
        "the magenta bottom band survived --crop-bottom: red {bottom_red}"
    );
    let (side_red, _, _) = frame.pixel(BAND / 2, middle);
    assert!(
        side_red > HIGH,
        "the red left band was cut without --crop-left: red {side_red}"
    );
}

#[test]
fn auto_crop_removes_the_bars_and_fits_the_active_area() {
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("barred.mkv");
    barred_clip(&clip);

    let imp = create(
        dir.path(),
        "auto",
        &clip,
        &["--auto-crop", "--raster", RASTER_2K],
    );
    let frame = packaged_frame(&imp, 0);
    assert_eq!(
        (frame.width, frame.height),
        (RASTER_2K_WIDTH, RASTER_2K_HEIGHT)
    );

    // the 1920x800 active area scaled to the 2048 wide raster
    let expected_height = ACTIVE_HEIGHT * RASTER_2K_WIDTH / WIDTH;
    let expected_top = (RASTER_2K_HEIGHT - expected_height) / 2;
    let (first, last, height) = frame.content_rows();
    assert!(
        height.abs_diff(expected_height) <= FIT_TOLERANCE,
        "the picture is {height} rows tall, not the {expected_height} the active area fits to \
         (rows {first} to {last})"
    );
    assert!(
        first.abs_diff(expected_top) <= FIT_TOLERANCE,
        "the picture starts at row {first}, not the {expected_top} centring it puts it at"
    );

    // the bars are gone, so the active area reached the full raster width
    for column in [FIT_TOLERANCE, RASTER_2K_WIDTH - 1 - FIT_TOLERANCE] {
        let (red, green, blue) = frame.pixel(column, RASTER_2K_HEIGHT / 2);
        assert!(
            red > HIGH && green > HIGH && blue > HIGH,
            "column {column} is not picture: {red},{green},{blue}. The bars were kept, so the \
             1920x1080 source was pillarboxed instead of the 1920x{ACTIVE_HEIGHT} active area \
             being scaled up"
        );
    }
    // the bar rows themselves are black, above and below the fitted picture
    for row in [FIT_TOLERANCE, RASTER_2K_HEIGHT - 1 - FIT_TOLERANCE] {
        let (_, green, _) = frame.pixel(RASTER_2K_WIDTH / 2, row);
        assert!(green < LOW, "row {row} is not black: green {green}");
    }
}

#[test]
fn a_fill_crop_reaches_both_edges_where_the_fit_pillarboxes() {
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("white.mkv");
    build_clip(
        &clip,
        &format!("color=c=white:s={WIDTH}x{HEIGHT}:r={FPS}"),
        "format=gbrp",
        FRAMES,
        "gbrp",
    );

    // 1920x1080 into a 2048x1080 raster leaves a black pillar of (2048-1920)/2
    let fitted = create(dir.path(), "fitted", &clip, &["--raster", RASTER_2K]);
    let frame = packaged_frame(&fitted, 0);
    let pillar = (RASTER_2K_WIDTH - WIDTH) / 2;
    let (_, green, _) = frame.pixel(pillar / 2, RASTER_2K_HEIGHT / 2);
    assert!(
        green < LOW,
        "without --fill-crop there has to be a {pillar} pixel black pillar: green {green}"
    );

    let filled = create(
        dir.path(),
        "filled",
        &clip,
        &["--fill-crop", "--raster", RASTER_2K],
    );
    let frame = packaged_frame(&filled, 0);
    assert_eq!(
        (frame.width, frame.height),
        (RASTER_2K_WIDTH, RASTER_2K_HEIGHT)
    );
    let (left, right, top, bottom) = frame.bright_bounds(MARKER_THRESHOLD);
    assert_eq!(
        (left, right),
        (0, RASTER_2K_WIDTH - 1),
        "--fill-crop has to reach both side edges, not {left}..{right}"
    );
    assert!(
        top <= EXACT_TOLERANCE && RASTER_2K_HEIGHT - 1 - bottom <= FIT_TOLERANCE,
        "--fill-crop has to reach top and bottom, not rows {top}..{bottom} of {RASTER_2K_HEIGHT}"
    );
}

/// Rows the pattern scrolls between the two fields of one frame.
const SCROLL_ROWS_PER_FRAME: u32 = 40;

/// Row-to-row difference the combed source packages at, and what yadif brings
/// it down to. Measured on the decoded picture track file: 170 codes of 4095
/// combed, 4.5 deinterlaced.
const COMBED_ROW_DIFFERENCE: f64 = 120.0;
const DEINTERLACED_ROW_DIFFERENCE: f64 = 20.0;

#[test]
fn deinterlace_takes_the_combing_out_of_the_packaged_frame() {
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("combed.mkv");
    // a testsrc scrolled 40 rows a frame, then woven pair by pair, so every
    // other row of the result is 40 rows out of step with its neighbours
    build_clip(
        &clip,
        &format!("testsrc=s={WIDTH}x{}:r={}", HEIGHT * 2, FPS * 2),
        &format!(
            "crop={WIDTH}:{HEIGHT}:0:'mod(n*{SCROLL_ROWS_PER_FRAME},{HEIGHT})',\
             tinterlace=mode=interleave_top,format=gbrp"
        ),
        FRAMES,
        "gbrp",
    );

    let combed =
        packaged_frame(&create(dir.path(), "combed", &clip, &[]), 0).row_to_row_difference();
    let deinterlaced = packaged_frame(&create(dir.path(), "clean", &clip, &["--deinterlace"]), 0)
        .row_to_row_difference();
    eprintln!("row-to-row difference: combed {combed:.1}, deinterlaced {deinterlaced:.1}");

    assert!(
        combed > COMBED_ROW_DIFFERENCE,
        "the source is not combed enough to measure a deinterlace: {combed:.1} codes between \
         adjacent rows"
    );
    assert!(
        deinterlaced < DEINTERLACED_ROW_DIFFERENCE,
        "--deinterlace left {deinterlaced:.1} codes between adjacent rows, against \
         {combed:.1} without it"
    );
}

/// Noise the source carries, on the noise filter's 0 to 100 strength scale.
/// hqdn3d at its defaults is a grain filter: at a strength of 40 it takes only
/// 3 percent off, so a heavier source would measure nothing.
const NOISE_SIGMA: u32 = 5;
const NOISE_SEED: u32 = 1;

/// Pixel-to-pixel difference the noisy flat frame packages at, and what hqdn3d
/// brings it down to. Measured on the decoded picture track file: 39 codes of
/// 4095 noisy, 12.5 denoised. The same clip without the noise packages at 0.0,
/// so both numbers are the source's noise and none of them is the encode's.
const NOISY_PIXEL_DIFFERENCE: f64 = 30.0;
const DENOISED_PIXEL_DIFFERENCE: f64 = 18.0;

#[test]
fn denoise_takes_the_grain_out_of_the_packaged_frame() {
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("noisy.mkv");
    // a flat grey frame, so every difference between neighbouring pixels in the
    // packaged picture is noise
    build_clip(
        &clip,
        &format!("color=c=gray:s={WIDTH}x{HEIGHT}:r={FPS}"),
        // the filter seeds itself from the clock without all_seed, and these
        // assertions carry measured numbers
        &format!("format=yuv444p,noise=alls={NOISE_SIGMA}:allf=t+u:all_seed={NOISE_SEED}"),
        DENOISE_FRAMES,
        "yuv444p",
    );

    // hqdn3d's temporal pass needs frames behind it, so measure the last one
    let last = DENOISE_FRAMES - 1;
    let noisy =
        packaged_frame(&create(dir.path(), "noisy", &clip, &[]), last).pixel_to_pixel_difference();
    let denoised = packaged_frame(&create(dir.path(), "denoised", &clip, &["--denoise"]), last)
        .pixel_to_pixel_difference();
    eprintln!("pixel-to-pixel difference: noisy {noisy:.1}, denoised {denoised:.1}");

    assert!(
        noisy > NOISY_PIXEL_DIFFERENCE,
        "the source is not noisy enough to measure a denoise: {noisy:.1} codes between \
         neighbouring pixels"
    );
    assert!(
        denoised < DENOISED_PIXEL_DIFFERENCE,
        "--denoise left {denoised:.1} codes between neighbouring pixels of a flat frame, \
         against {noisy:.1} without it"
    );
}

/// Side of the white square the rotate and flip sources carry.
const MARKER: u32 = 200;

#[test]
fn rotate_90_puts_the_top_left_marker_at_the_top_right() {
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("marked.mkv");
    marked_clip(&clip, MARKER);

    let imp = create(dir.path(), "turned", &clip, &["--rotate", "90"]);
    let frame = packaged_frame(&imp, 0);
    assert_eq!((frame.width, frame.height), (WIDTH, HEIGHT));

    // a quarter turn makes the picture 1080x1920, which fits the 1920x1080
    // raster at 606x1080 and is centred with a 656 pixel pillar each side
    let picture_width = 606u32;
    let pillar = (WIDTH - picture_width) / 2;
    let scale = picture_width as f64 / HEIGHT as f64;
    let marker_width = (MARKER as f64 * scale) as u32;

    let (left, right, top, bottom) = frame.bright_bounds(MARKER_THRESHOLD);
    eprintln!("marker after --rotate 90: {left}..{right} by {top}..{bottom}");
    assert!(
        right.abs_diff(pillar + picture_width - 1) <= FIT_TOLERANCE,
        "the marker's right edge is at {right}, not the {} the turned picture's right edge is at",
        pillar + picture_width - 1
    );
    assert!(
        left.abs_diff(pillar + picture_width - marker_width) <= FIT_TOLERANCE,
        "the marker is {} wide, not the {marker_width} a {MARKER} pixel square scales to",
        right - left + 1
    );
    assert_eq!(top, 0, "the marker has to stay against the top edge");
    assert!(
        bottom.abs_diff(marker_width - 1) <= FIT_TOLERANCE,
        "the marker runs to row {bottom}, not the {} it scales to",
        marker_width - 1
    );

    // the corner the marker came from is black, so this is a turn and not a copy
    let (_, green, _) = frame.pixel(pillar + MARKER / 2, MARKER / 2);
    assert!(
        green < LOW,
        "the top-left of the turned picture is not black: green {green}"
    );
}

#[test]
fn a_horizontal_flip_puts_the_top_left_marker_at_the_top_right() {
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("marked.mkv");
    marked_clip(&clip, MARKER);

    let imp = create(dir.path(), "flipped", &clip, &["--flip", "horizontal"]);
    let frame = packaged_frame(&imp, 0);
    assert_eq!((frame.width, frame.height), (WIDTH, HEIGHT));

    // a flip neither scales nor pads, so the marker mirrors onto the far edge
    let (left, right, top, bottom) = frame.bright_bounds(MARKER_THRESHOLD);
    eprintln!("marker after --flip horizontal: {left}..{right} by {top}..{bottom}");
    assert!(
        left.abs_diff(WIDTH - MARKER) <= EXACT_TOLERANCE && right == WIDTH - 1,
        "the marker is at columns {left}..{right}, not the {}..{} a mirror puts it at",
        WIDTH - MARKER,
        WIDTH - 1
    );
    assert!(
        top == 0 && bottom.abs_diff(MARKER - 1) <= EXACT_TOLERANCE,
        "a horizontal flip must not move the marker off the top: rows {top}..{bottom}"
    );
}

/// A 2x2x2 .cube whose entries are the cube corners with red and blue swapped.
/// The entries run with the red index changing fastest, so line n holds the
/// output for input (n & 1, (n >> 1) & 1, (n >> 2) & 1).
const SWAP_RED_AND_BLUE_CUBE: &str = "LUT_3D_SIZE 2\n\
    0.0 0.0 0.0\n0.0 0.0 1.0\n0.0 1.0 0.0\n0.0 1.0 1.0\n\
    1.0 0.0 0.0\n1.0 0.0 1.0\n1.0 1.0 0.0\n1.0 1.0 1.0\n";

#[test]
fn a_source_lut_transforms_the_packaged_picture() {
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("red.mkv");
    build_clip(
        &clip,
        &format!("color=c=red:s={WIDTH}x{HEIGHT}:r={FPS}"),
        "format=gbrp",
        FRAMES,
        "gbrp",
    );
    let lut = dir.path().join("swap_red_and_blue.cube");
    std::fs::write(&lut, SWAP_RED_AND_BLUE_CUBE).unwrap();
    let lut_argument = lut.to_string_lossy().to_string();

    let plain = packaged_frame(&create(dir.path(), "plain", &clip, &[]), 0);
    let (red, green, blue) = plain.pixel(WIDTH / 2, HEIGHT / 2);
    assert!(
        red > HIGH && green < LOW && blue < LOW,
        "the source has to package as red without a LUT: {red},{green},{blue}"
    );

    let imp = create(
        dir.path(),
        "through_lut",
        &clip,
        &["--source-lut", &lut_argument],
    );
    let frame = packaged_frame(&imp, 0);
    let (red, green, blue) = frame.pixel(WIDTH / 2, HEIGHT / 2);
    assert!(
        blue > HIGH && red < LOW && green < LOW,
        "the LUT swaps red and blue, so the packaged picture has to be near {FULL_SCALE} blue: \
         {red},{green},{blue}"
    );
}
