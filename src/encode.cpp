#include <spdlog/spdlog.h>
#include <algorithm>
#include <array>
#include <cstdio>
#include <filesystem>
#include <stdexcept>
#include <string>

#include "imfwizard/encode.h"

namespace fs = std::filesystem;

namespace imfwizard
{

ImageFormat detect_format(const fs::path& file)
{
  auto ext = file.extension().string();
  std::transform(ext.begin(), ext.end(), ext.begin(), ::tolower);

  if(ext == ".dpx")
    return ImageFormat::DPX;
  if(ext == ".tif" || ext == ".tiff")
    return ImageFormat::TIFF;
  if(ext == ".exr")
    return ImageFormat::EXR;
  if(ext == ".png")
    return ImageFormat::PNG;
  if(ext == ".jpg" || ext == ".jpeg")
    return ImageFormat::JPEG;
  if(ext == ".bmp")
    return ImageFormat::BMP;
  if(ext == ".j2c" || ext == ".j2k" || ext == ".jp2")
    return ImageFormat::J2K;
  return ImageFormat::Unknown;
}

ImageFormat detect_sequence_format(const fs::path& dir)
{
  for(const auto& entry : fs::directory_iterator(dir))
  {
    if(!entry.is_regular_file())
      continue;
    auto fmt = detect_format(entry.path());
    if(fmt != ImageFormat::Unknown)
      return fmt;
  }
  return ImageFormat::Unknown;
}

uint32_t count_frames(const fs::path& dir)
{
  uint32_t count = 0;
  for(const auto& entry : fs::directory_iterator(dir))
  {
    if(!entry.is_regular_file())
      continue;
    auto fmt = detect_format(entry.path());
    if(fmt != ImageFormat::Unknown)
      ++count;
  }
  return count;
}

static std::string find_encoder()
{
  std::string cmd = "grk_compress";
  std::string check = std::string("which ") + cmd + " >/dev/null 2>&1";
#ifdef _WIN32
  check = std::string("where ") + cmd + " >NUL 2>&1";
#endif
  if(system(check.c_str()) == 0)
    return cmd;
  return "";
}

static std::string format_to_input_flag(ImageFormat fmt)
{
  switch(fmt)
  {
    case ImageFormat::DPX:
      return "dpx";
    case ImageFormat::TIFF:
      return "tif";
    case ImageFormat::EXR:
      return "exr";
    case ImageFormat::PNG:
      return "png";
    case ImageFormat::BMP:
      return "bmp";
    case ImageFormat::JPEG:
      return "jpg";
    default:
      return "";
  }
}

EncodeResult encode_to_j2k(const EncodeOptions& opts)
{
  EncodeResult result;
  result.output_dir = opts.output_dir;

  // If input is already J2K, just copy/symlink
  ImageFormat fmt = opts.format;
  if(fmt == ImageFormat::Unknown)
    fmt = detect_sequence_format(opts.input_dir);

  if(fmt == ImageFormat::J2K)
  {
    // Already encoded — just point to the input directory
    result.output_dir = opts.input_dir;
    result.frame_count = count_frames(opts.input_dir);
    result.success = true;
    spdlog::info("Input already J2K ({} frames), skipping encode", result.frame_count);
    return result;
  }

  if(fmt == ImageFormat::Unknown)
  {
    result.error = "Cannot detect image format in: " + opts.input_dir.string();
    return result;
  }

  // Find encoder
  std::string encoder = find_encoder();
  if(encoder.empty())
  {
    result.error = "No J2K encoder found. Install grk_compress (grok)";
    return result;
  }

  spdlog::info("Using encoder: {}", encoder);
  fs::create_directories(opts.output_dir);

  // Count input files
  std::vector<fs::path> input_files;
  for(const auto& entry : fs::directory_iterator(opts.input_dir))
  {
    if(!entry.is_regular_file())
      continue;
    if(detect_format(entry.path()) == fmt)
      input_files.push_back(entry.path());
  }
  std::sort(input_files.begin(), input_files.end());

  result.frame_count = static_cast<uint32_t>(input_files.size());
  spdlog::info("Encoding {} frames ({}) -> J2K", result.frame_count, format_to_input_flag(fmt));

  // Use batch mode for encoding (much faster, and compatible with grok plugin builds)
  std::string cmd = encoder;
  if(opts.cinema_profile)
    cmd += " -cinema2K 24";
  cmd += " -batch_src " + opts.input_dir.string();
  cmd += " -a " + opts.output_dir.string();
  cmd += " -O J2K";
  if(opts.num_threads > 0)
    cmd += " -H " + std::to_string(opts.num_threads);

  spdlog::debug("Command: {}", cmd);
  int ret = system(cmd.c_str());
  if(ret != 0)
  {
    result.error = "Batch J2K encoding failed (exit code " + std::to_string(ret) + ")";
    return result;
  }

  // Verify output count
  uint32_t encoded = 0;
  for(const auto& entry : fs::directory_iterator(opts.output_dir))
  {
    auto ext = entry.path().extension().string();
    std::transform(ext.begin(), ext.end(), ext.begin(), ::tolower);
    if(ext == ".j2k" || ext == ".j2c")
      ++encoded;
  }
  spdlog::info("Encoded {} frames", encoded);

  result.success = true;
  spdlog::info("Encoding complete: {} frames in {}", encoded, opts.output_dir.string());
  return result;
}

} // namespace imfwizard
