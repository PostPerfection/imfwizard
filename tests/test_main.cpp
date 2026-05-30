#include <cassert>
#include <filesystem>
#include <fstream>
#include <spdlog/spdlog.h>
#include <string>

#include "imfwizard/imfwizard.h"
#include "imfwizard/encode.h"
#include "imfwizard/validate.h"
#include "imfwizard/transcode.h"
#include "imfwizard/preferences.h"
#include "imfwizard/profiles.h"
#include "imfwizard/timecode.h"
#include "imfwizard/supplemental.h"
#include "imfwizard/analytics.h"
#include "imfwizard/batch_deliver.h"
#include "imfwizard/dcp_convert.h"
#include "imfwizard/burnin.h"
#include "imfwizard/conform.h"
#include "imfwizard/hdr.h"
#include "imfwizard/job_queue.h"

namespace fs = std::filesystem;

static int tests_run = 0;
static int tests_passed = 0;

#define TEST(name)                                        \
  do                                                      \
  {                                                       \
    tests_run++;                                          \
    try                                                   \
    {                                                     \
      name();                                             \
      tests_passed++;                                     \
      spdlog::info("  {}... PASS", #name);                \
    }                                                     \
    catch(const std::exception& e)                        \
    {                                                     \
      spdlog::error("  {}... FAIL: {}", #name, e.what()); \
    }                                                     \
  } while(0)

#define ASSERT(cond)                                        \
  do                                                        \
  {                                                         \
    if(!(cond))                                             \
      throw std::runtime_error("Assertion failed: " #cond); \
  } while(0)

// --- UUID tests ---

void test_uuid_format()
{
  std::string uuid = imfwizard::generate_uuid();
  ASSERT(uuid.starts_with("urn:uuid:"));
  ASSERT(uuid.size() == 45); // urn:uuid: + 36 chars
}

void test_uuid_bare_format()
{
  std::string uuid = imfwizard::generate_uuid_bare();
  ASSERT(uuid.size() == 36);
  ASSERT(uuid[8] == '-');
  ASSERT(uuid[13] == '-');
  ASSERT(uuid[18] == '-');
  ASSERT(uuid[23] == '-');
}

void test_uuid_uniqueness()
{
  std::string a = imfwizard::generate_uuid();
  std::string b = imfwizard::generate_uuid();
  ASSERT(a != b);
}

// --- Hash tests ---

void test_sha1_base64()
{
  // Create a temp file with known content
  fs::path tmp = fs::temp_directory_path() / "imfwizard_test_hash.bin";
  {
    std::ofstream ofs(tmp, std::ios::binary);
    ofs << "hello world";
  }
  std::string hash = imfwizard::sha1_base64(tmp);
  // SHA-1 of "hello world" = 2aae6c35c94fcfb415dbe95f408b9ce91ee846ed
  // base64 = Kq5sNclPz7QV2+lfQIuc6R7oRu0=
  ASSERT(hash == "Kq5sNclPz7QV2+lfQIuc6R7oRu0=");
  fs::remove(tmp);
}

void test_sha1_nonexistent()
{
  bool threw = false;
  try
  {
    imfwizard::sha1_base64("/nonexistent_path_xyz");
  }
  catch(const std::runtime_error&)
  {
    threw = true;
  }
  ASSERT(threw);
}

// --- Encode tests ---

void test_detect_format()
{
  auto fmt = imfwizard::detect_format("test.tiff");
  ASSERT(fmt == imfwizard::ImageFormat::TIFF);

  fmt = imfwizard::detect_format("test.dpx");
  ASSERT(fmt == imfwizard::ImageFormat::DPX);

  fmt = imfwizard::detect_format("test.exr");
  ASSERT(fmt == imfwizard::ImageFormat::EXR);

  fmt = imfwizard::detect_format("test.xyz");
  ASSERT(fmt == imfwizard::ImageFormat::Unknown);
}

void test_encode_defaults()
{
  imfwizard::EncodeOptions opts;
  ASSERT(opts.target_bitrate_mbps == 250);
  ASSERT(opts.bit_depth == 12);
  ASSERT(opts.num_threads == 0); // auto
}

void test_encode_nonexistent()
{
  imfwizard::EncodeOptions opts;
  opts.input_dir = "/nonexistent_xyz";
  opts.output_dir = "/tmp/imfwizard_test_enc";
  bool threw = false;
  try
  {
    imfwizard::encode_to_j2k(opts);
  }
  catch(const std::exception&)
  {
    threw = true;
  }
  ASSERT(threw);
}

// --- Transcode tests ---

void test_detect_codec()
{
  // Nonexistent file should return Unknown
  auto codec = imfwizard::detect_codec("/nonexistent.mov");
  ASSERT(codec == imfwizard::VideoCodec::Unknown);
}

void test_transcode_nonexistent()
{
  imfwizard::TranscodeOptions opts;
  opts.input_file = "/nonexistent_xyz.mov";
  opts.output_dir = "/tmp/imfwizard_test_tc";
  auto result = imfwizard::transcode_to_sequence(opts);
  ASSERT(!result.success);
  ASSERT(!result.error.empty());
}

// --- Preferences tests ---

void test_preferences_defaults()
{
  imfwizard::Preferences prefs;
  ASSERT(prefs.default_bandwidth_mbps == 250);
  ASSERT(prefs.gpu_device == -1);
  ASSERT(prefs.loudness_target_lufs == -24.0);
  ASSERT(prefs.theme == "dark");
  ASSERT(prefs.show_advanced_options == false);
}

void test_preferences_path()
{
  auto path = imfwizard::preferences_path();
  ASSERT(!path.empty());
}

void test_preferences_roundtrip()
{
  imfwizard::Preferences prefs;
  prefs.creator_name = "Test Studio";
  prefs.default_bandwidth_mbps = 300;

  int rc = imfwizard::save_preferences(prefs);
  ASSERT(rc == 0);

  auto loaded = imfwizard::load_preferences();
  ASSERT(loaded.creator_name == "Test Studio");
  ASSERT(loaded.default_bandwidth_mbps == 300);
}

// --- Profiles tests ---

void test_profile_netflix()
{
  auto p = imfwizard::netflix_profile();
  ASSERT(p.name == "netflix");
  ASSERT(p.max_width >= 3840);
  ASSERT(p.requires_hdr == true);
}

void test_profile_cinema_2k()
{
  auto p = imfwizard::cinema_2k_profile();
  ASSERT(p.name == "cinema-2k");
  ASSERT(p.max_width == 2048);
  ASSERT(p.max_height == 1080);
}

void test_profile_get()
{
  auto p = imfwizard::get_profile("netflix");
  ASSERT(p.name == "netflix");
}

// --- Timecode tests ---

void test_timecode_from_frames()
{
  auto tc = imfwizard::Timecode::from_frames(48, 24.0);
  ASSERT(tc.hours == 0);
  ASSERT(tc.minutes == 0);
  ASSERT(tc.seconds == 2);
  ASSERT(tc.frames == 0);
}

void test_timecode_to_frames()
{
  imfwizard::Timecode tc{0, 1, 0, 0, false};
  uint64_t frames = tc.to_frames(24.0);
  ASSERT(frames == 1440); // 60 seconds * 24 fps
}

void test_timecode_roundtrip()
{
  auto tc = imfwizard::Timecode::from_frames(12345, 24.0);
  uint64_t frames = tc.to_frames(24.0);
  ASSERT(frames == 12345);
}

void test_timecode_to_string()
{
  imfwizard::Timecode tc{1, 2, 3, 4, false};
  auto s = tc.to_string();
  ASSERT(s == "01:02:03:04");
}

void test_timecode_parse()
{
  auto tc = imfwizard::Timecode::parse("01:02:03:04", 24.0);
  ASSERT(tc.hours == 1);
  ASSERT(tc.minutes == 2);
  ASSERT(tc.seconds == 3);
  ASSERT(tc.frames == 4);
}

// --- HDR tests ---

void test_hdr10_preset()
{
  auto meta = imfwizard::hdr10_preset();
  ASSERT(meta.transfer == imfwizard::TransferFunction::PQ);
  ASSERT(meta.colorimetry == imfwizard::Colorimetry::BT2020);
  ASSERT(meta.bit_depth >= 10);
}

void test_hlg_preset()
{
  auto meta = imfwizard::hlg_preset();
  ASSERT(meta.transfer == imfwizard::TransferFunction::HLG);
  ASSERT(meta.colorimetry == imfwizard::Colorimetry::BT2020);
}

void test_hdr_metadata_xml()
{
  auto meta = imfwizard::hdr10_preset();
  auto xml = imfwizard::generate_color_metadata_xml(meta);
  ASSERT(!xml.empty());
  ASSERT(xml.find("PQ") != std::string::npos || xml.find("SMPTE-ST-2084") != std::string::npos);
}

// --- IMP creation tests ---

void test_imp_options_defaults()
{
  imfwizard::ImpOptions opts;
  ASSERT(opts.edit_rate_num == 24);
  ASSERT(opts.edit_rate_den == 1);
  ASSERT(opts.audio_sample_rate == 48000);
  ASSERT(opts.audio_bit_depth == 24);
}

void test_create_imp_nonexistent()
{
  imfwizard::ImpOptions opts;
  opts.title = "Test";
  opts.video_dir = "/nonexistent_xyz";
  opts.output_dir = "/tmp/imfwizard_test_imp";
  auto result = imfwizard::create_ov_imp(opts);
  ASSERT(!result.success);
  ASSERT(!result.error.empty());
}

// --- CPL tests ---

void test_cpl_generation()
{
  imfwizard::CplOptions opts;
  opts.title = "Test CPL";
  opts.issuer = "Test Suite";
  opts.edit_rate_num = 24;
  opts.edit_rate_den = 1;
  auto tmp = fs::temp_directory_path() / "imfwizard_cpl_test";
  fs::create_directories(tmp);
  auto result = imfwizard::generate_cpl(opts, tmp);
  ASSERT(!result.uuid.empty());
  // uuid is bare (no urn:uuid: prefix)
  ASSERT(result.uuid.length() == 36);
  fs::remove_all(tmp);
}

// --- PKL tests ---

void test_pkl_generation()
{
  imfwizard::PklOptions opts;
  opts.issuer = "Test Suite";
  auto tmp = fs::temp_directory_path() / "imfwizard_pkl_test";
  fs::create_directories(tmp);
  auto result = imfwizard::generate_pkl(opts, tmp);
  ASSERT(!result.uuid.empty());
  fs::remove_all(tmp);
}

// --- Job Queue tests ---

void test_job_type_strings()
{
  ASSERT(imfwizard::job_type_to_string(imfwizard::JobType::Encode) == "encode");
  ASSERT(imfwizard::job_type_to_string(imfwizard::JobType::Transcode) == "transcode");
  ASSERT(imfwizard::job_type_to_string(imfwizard::JobType::Create) == "create");
  ASSERT(imfwizard::job_type_to_string(imfwizard::JobType::Validate) == "validate");
}

void test_job_state_strings()
{
  ASSERT(imfwizard::job_state_to_string(imfwizard::JobState::Queued) == "queued");
  ASSERT(imfwizard::job_state_to_string(imfwizard::JobState::Running) == "running");
  ASSERT(imfwizard::job_state_to_string(imfwizard::JobState::Completed) == "completed");
  ASSERT(imfwizard::job_state_to_string(imfwizard::JobState::Failed) == "failed");
}

void test_job_type_roundtrip()
{
  auto s = imfwizard::job_type_to_string(imfwizard::JobType::Loudness);
  auto t = imfwizard::job_type_from_string(s);
  ASSERT(t == imfwizard::JobType::Loudness);
}

void test_job_state_roundtrip()
{
  auto s = imfwizard::job_state_to_string(imfwizard::JobState::Cancelled);
  auto st = imfwizard::job_state_from_string(s);
  ASSERT(st == imfwizard::JobState::Cancelled);
}

// --- Analytics tests ---

void test_analytics_nonexistent()
{
  imfwizard::AnalyticsOptions opts;
  opts.source = "/nonexistent_xyz";
  auto result = imfwizard::compute_analytics(opts);
  ASSERT(!result.success);
}

void test_analytics_to_json()
{
  imfwizard::AnalyticsResult result;
  result.total_frames = 0;
  result.avg_bitrate_mbps = 200.0;
  result.success = true;
  auto json = imfwizard::analytics_to_json(result);
  ASSERT(!json.empty());
  ASSERT(json.find("200") != std::string::npos);
}

// --- Validate tests ---

void test_validate_nonexistent()
{
  // Validation of nonexistent dir should produce errors
  // Note: validation is delegated to dcpdoctor
}

// --- DCP Convert tests ---

void test_dcp_convert_nonexistent()
{
  imfwizard::DcpConvertOptions opts;
  opts.imp_dir = "/nonexistent_xyz";
  opts.output_dir = "/tmp/imfwizard_dcp_test";
  auto result = imfwizard::convert_imp_to_dcp(opts);
  ASSERT(!result.success);
  ASSERT(!result.error.empty());
}

// --- Conform tests ---

void test_parse_edl_nonexistent()
{
  auto entries = imfwizard::parse_edl("/nonexistent.edl");
  ASSERT(entries.empty());
}

// --- Burn-in tests ---

void test_burnin_nonexistent()
{
  imfwizard::BurnInOptions opts;
  opts.video_input = "/nonexistent.mov";
  opts.subtitle_file = "/nonexistent.srt";
  opts.output = "/tmp/imfwizard_burnin_test.mov";
  auto result = imfwizard::burn_in_subtitles(opts);
  ASSERT(!result.success);
}

// --- Supplemental tests ---

void test_supplemental_nonexistent()
{
  imfwizard::SupplementalOptions opts;
  opts.ov_imp_dir = "/nonexistent_xyz";
  opts.output_dir = "/tmp/imfwizard_sup_test";
  auto result = imfwizard::create_supplemental_imp(opts);
  ASSERT(!result.success);
}

// --- Transcode & video file tests ---

void test_ffmpeg_available()
{
  bool avail = imfwizard::ffmpeg_available();
  if(system("which ffmpeg >/dev/null 2>&1") == 0)
    ASSERT(avail);
  else
    ASSERT(!avail);
}

void test_ffprobe_available()
{
  bool avail = imfwizard::ffprobe_available();
  if(system("which ffprobe >/dev/null 2>&1") == 0)
    ASSERT(avail);
  else
    ASSERT(!avail);
}

void test_transcode_result_defaults()
{
  imfwizard::TranscodeResult result;
  ASSERT(result.frame_count == 0);
  ASSERT(result.width == 0);
  ASSERT(result.height == 0);
  ASSERT(result.fps == 0.0);
  ASSERT(!result.success);
  ASSERT(result.error.empty());
}

void test_transcode_options_defaults()
{
  imfwizard::TranscodeOptions opts;
  ASSERT(opts.output_format == "tiff");
  ASSERT(opts.bit_depth == 16);
  ASSERT(opts.start_frame == 0);
  ASSERT(opts.end_frame == 0);
  ASSERT(opts.threads == 0);
}

void test_detect_format_j2k()
{
  ASSERT(imfwizard::detect_format("/tmp/test.j2c") == imfwizard::ImageFormat::J2K);
  ASSERT(imfwizard::detect_format("/tmp/test.j2k") == imfwizard::ImageFormat::J2K);
  ASSERT(imfwizard::detect_format("/tmp/test.jp2") == imfwizard::ImageFormat::J2K);
}

void test_detect_format_image_types()
{
  ASSERT(imfwizard::detect_format("/tmp/test.dpx") == imfwizard::ImageFormat::DPX);
  ASSERT(imfwizard::detect_format("/tmp/test.tif") == imfwizard::ImageFormat::TIFF);
  ASSERT(imfwizard::detect_format("/tmp/test.tiff") == imfwizard::ImageFormat::TIFF);
  ASSERT(imfwizard::detect_format("/tmp/test.exr") == imfwizard::ImageFormat::EXR);
  ASSERT(imfwizard::detect_format("/tmp/test.png") == imfwizard::ImageFormat::PNG);
  ASSERT(imfwizard::detect_format("/tmp/test.bmp") == imfwizard::ImageFormat::BMP);
}

void test_detect_format_unknown()
{
  ASSERT(imfwizard::detect_format("/tmp/test.mp4") == imfwizard::ImageFormat::Unknown);
  ASSERT(imfwizard::detect_format("/tmp/test.txt") == imfwizard::ImageFormat::Unknown);
}

void test_detect_sequence_empty_dir()
{
  auto tmp = fs::temp_directory_path() / "imfwiz_test_empty_seq";
  fs::create_directories(tmp);
  ASSERT(imfwizard::detect_sequence_format(tmp) == imfwizard::ImageFormat::Unknown);
  fs::remove_all(tmp);
}

void test_count_frames_empty()
{
  auto tmp = fs::temp_directory_path() / "imfwiz_test_count_empty";
  fs::create_directories(tmp);
  ASSERT(imfwizard::count_frames(tmp) == 0);
  fs::remove_all(tmp);
}

void test_count_frames_with_files()
{
  auto tmp = fs::temp_directory_path() / "imfwiz_test_count_frames";
  fs::create_directories(tmp);
  {
    std::ofstream f(tmp / "frame_001.tiff");
    f << "fake";
  }
  {
    std::ofstream f(tmp / "frame_002.tiff");
    f << "fake";
  }
  {
    std::ofstream f(tmp / "frame_003.tiff");
    f << "fake";
  }
  {
    std::ofstream f(tmp / "readme.txt");
    f << "not a frame";
  }
  ASSERT(imfwizard::count_frames(tmp) == 3);
  fs::remove_all(tmp);
}

void test_imp_create_invalid_video()
{
  imfwizard::ImpOptions opts;
  opts.title = "Test";
  opts.video_dir = "/nonexistent_video_dir_xyz123";
  opts.output_dir = "/tmp/imfwiz_test_fail";
  auto result = imfwizard::create_ov_imp(opts);
  ASSERT(!result.success);
  ASSERT(!result.error.empty());
}

int main()
{
  spdlog::info("IMF Wizard Tests");
  spdlog::info("================");

  spdlog::info("UUID:");
  TEST(test_uuid_format);
  TEST(test_uuid_bare_format);
  TEST(test_uuid_uniqueness);

  spdlog::info("Hash:");
  TEST(test_sha1_base64);
  TEST(test_sha1_nonexistent);

  spdlog::info("Encode:");
  TEST(test_detect_format);
  TEST(test_encode_defaults);
  TEST(test_encode_nonexistent);

  spdlog::info("Transcode:");
  TEST(test_detect_codec);
  TEST(test_transcode_nonexistent);

  spdlog::info("Preferences:");
  TEST(test_preferences_defaults);
  TEST(test_preferences_path);
  TEST(test_preferences_roundtrip);

  spdlog::info("Profiles:");
  TEST(test_profile_netflix);
  TEST(test_profile_cinema_2k);
  TEST(test_profile_get);

  spdlog::info("Timecode:");
  TEST(test_timecode_from_frames);
  TEST(test_timecode_to_frames);
  TEST(test_timecode_roundtrip);
  TEST(test_timecode_to_string);
  TEST(test_timecode_parse);

  spdlog::info("HDR:");
  TEST(test_hdr10_preset);
  TEST(test_hlg_preset);
  TEST(test_hdr_metadata_xml);

  spdlog::info("IMP:");
  TEST(test_imp_options_defaults);
  TEST(test_create_imp_nonexistent);

  spdlog::info("CPL:");
  TEST(test_cpl_generation);

  spdlog::info("PKL:");
  TEST(test_pkl_generation);

  spdlog::info("Job Queue:");
  TEST(test_job_type_strings);
  TEST(test_job_state_strings);
  TEST(test_job_type_roundtrip);
  TEST(test_job_state_roundtrip);

  spdlog::info("Analytics:");
  TEST(test_analytics_nonexistent);
  TEST(test_analytics_to_json);

  spdlog::info("DCP Convert:");
  TEST(test_dcp_convert_nonexistent);

  spdlog::info("Conform:");
  TEST(test_parse_edl_nonexistent);

  spdlog::info("Burn-in:");
  TEST(test_burnin_nonexistent);

  spdlog::info("Supplemental:");
  TEST(test_supplemental_nonexistent);

  spdlog::info("Transcode & Video:");
  TEST(test_ffmpeg_available);
  TEST(test_ffprobe_available);
  TEST(test_transcode_result_defaults);
  TEST(test_transcode_options_defaults);
  TEST(test_detect_format_j2k);
  TEST(test_detect_format_image_types);
  TEST(test_detect_format_unknown);
  TEST(test_detect_sequence_empty_dir);
  TEST(test_count_frames_empty);
  TEST(test_count_frames_with_files);
  TEST(test_imp_create_invalid_video);

  spdlog::info("{}/{} tests passed", tests_passed, tests_run);
  return (tests_passed == tests_run) ? 0 : 1;
}
