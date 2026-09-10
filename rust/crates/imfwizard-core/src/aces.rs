use std::path::Path;

// the ACES AP0 to AP1 and AP1 to Rec.709 matrices, each carrying the Bradford
// adaptation to D65. colorchannelmixer refuses a coefficient outside ±2, which
// the single AP0 to Rec.709 matrix needs, so the two run in series
const AP0_TO_AP1: [[f64; 3]; 3] = [
    [1.4514393161, -0.2365107469, -0.2149285693],
    [-0.0765537733, 1.1762296998, -0.0996759265],
    [0.0083161484, -0.0060324498, 0.9977163014],
];
const AP1_TO_REC709: [[f64; 3]; 3] = [
    [1.70505, -0.62179, -0.08326],
    [-0.13026, 1.14080, -0.01055],
    [-0.02400, -0.12897, 1.15297],
];

fn channel_mixer(matrix: &[[f64; 3]; 3]) -> String {
    let coefficients: Vec<String> = matrix
        .iter()
        // the fourth coefficient of each row is the alpha the output ignores
        .flat_map(|row| {
            row.iter()
                .map(|value| value.to_string())
                .chain(["0".into()])
        })
        .collect();
    format!("colorchannelmixer={}", coefficients.join(":"))
}

pub fn convert_ap0_to_rec709(input: &Path, output: &Path) -> Result<(), String> {
    let filter = format!(
        "format=gbrpf32le,{},{},zscale=transferin=linear:transfer=bt709,format=gbrp16le",
        channel_mixer(&AP0_TO_AP1),
        channel_mixer(&AP1_TO_REC709)
    );
    let converted = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-i"])
        .arg(input)
        .args(["-vf", &filter, "-c:v", "tiff"])
        .arg(output)
        .output()
        .map_err(|e| format!("Failed to run ffmpeg: {e}"))?;

    if !converted.status.success() {
        return Err(format!(
            "the ffmpeg ACES conversion failed: {}",
            String::from_utf8_lossy(&converted.stderr).trim()
        ));
    }

    Ok(())
}
