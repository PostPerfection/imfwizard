cmake_minimum_required(VERSION 3.25)
project(imfwizard
    VERSION 0.5.0
    DESCRIPTION "IMF Wizard — Interoperable Master Format package creator"
    LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# Python bindings require position-independent code for all static libs
option(BUILD_PYTHON_BINDINGS "Build Python bindings via SWIG" OFF)
if(BUILD_PYTHON_BINDINGS)
  set(CMAKE_POSITION_INDEPENDENT_CODE ON)
endif()

# Dependencies
if(WIN32)
    find_package(LibXml2 REQUIRED)
    find_package(OpenSSL REQUIRED)
    find_package(XercesC REQUIRED)
else()
    find_package(PkgConfig REQUIRED)
    pkg_check_modules(LIBXML2 REQUIRED IMPORTED_TARGET libxml-2.0)
    pkg_check_modules(OPENSSL REQUIRED IMPORTED_TARGET openssl)
    pkg_check_modules(XERCES REQUIRED IMPORTED_TARGET xerces-c)
endif()

# CLI11 (header-only)
add_subdirectory(extern/CLI11 EXCLUDE_FROM_ALL)

# spdlog
add_subdirectory(extern/spdlog EXCLUDE_FROM_ALL)

# asdcplib
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
add_subdirectory(extern/asdcplib EXCLUDE_FROM_ALL)

# dcpdoctor (validation/QC library)
add_subdirectory(extern/dcpdoctor EXCLUDE_FROM_ALL)

# Core library
add_library(imfwizard_core STATIC
    src/mxf_wrap.cpp
    src/cpl.cpp
    src/pkl.cpp
    src/assetmap.cpp
    src/hash.cpp
    src/signature.cpp
    src/imp.cpp
    src/info.cpp
    src/supplemental.cpp
    src/timed_text.cpp
    src/encode.cpp
    src/multi_cpl.cpp
    src/transcode.cpp
    src/dolby_vision.cpp
    src/hdr.cpp
    src/iab.cpp
    src/audio_lang.cpp
    src/watch.cpp
    src/extract.cpp
    src/report.cpp
    src/conform.cpp
    src/profiles.cpp
    src/preferences.cpp
    src/captions.cpp
    src/channel_map.cpp
    src/cloud.cpp
    src/hdr10plus.cpp
    src/accessibility.cpp
    src/watermark.cpp
    src/aspera.cpp
    src/uuid.cpp
    src/job_queue.cpp
    src/preview.cpp
    src/prores.cpp
    src/burnin.cpp
    src/dcp_convert.cpp
    src/batch_deliver.cpp
    src/analytics.cpp
    src/rest_api.cpp
    src/edl_import.cpp
    src/plugin.cpp
    src/atmos.cpp
    src/mca.cpp
    src/audio_desc.cpp
    src/lut.cpp
    src/aces.cpp
    src/cpl_annotation.cpp
    src/partial_version.cpp
    src/slate.cpp
    src/subtitle_retime.cpp
    src/sdi_output.cpp
    src/imp_diff.cpp
    src/otioz_import.cpp
    src/multi_node.cpp
    src/kdm_gen.cpp
    src/mxf_playback.cpp
    src/webhook.cpp
    src/shell_completion.cpp
    src/pdf_report.cpp
    src/subtitle_convert.cpp
    src/timecode.cpp)

target_include_directories(imfwizard_core PUBLIC
    ${CMAKE_SOURCE_DIR}/include
    ${CMAKE_SOURCE_DIR}/extern/asdcplib/src)

if(WIN32)
    target_compile_definitions(imfwizard_core PUBLIC KM_WIN32)
    target_link_libraries(imfwizard_core PUBLIC
        LibXml2::LibXml2
        OpenSSL::SSL OpenSSL::Crypto
        XercesC::XercesC
        spdlog::spdlog
        libas02
        libasdcp
        libkumu
        dcpdoctor_core
        ws2_32)
else()
    target_link_libraries(imfwizard_core PUBLIC
        PkgConfig::LIBXML2
        PkgConfig::OPENSSL
        PkgConfig::XERCES
        spdlog::spdlog
        libas02
        libasdcp
        libkumu
        dcpdoctor_core)
endif()

# CLI tool
add_executable(imfwizard src/main.cpp)
target_link_libraries(imfwizard PRIVATE imfwizard_core CLI11::CLI11)

# Tests
option(BUILD_TESTING "Build tests" ON)
if(BUILD_TESTING)
    enable_testing()
    add_executable(imfwizard_test tests/test_main.cpp)
    target_link_libraries(imfwizard_test PRIVATE imfwizard_core)
    add_test(NAME imfwizard_test COMMAND imfwizard_test)

    add_executable(imfwizard_integration_test tests/test_integration.cpp)
    target_link_libraries(imfwizard_integration_test PRIVATE imfwizard_core)
    add_test(NAME imfwizard_integration_test COMMAND imfwizard_integration_test)

    add_executable(imfwizard_job_queue_test tests/test_job_queue.cpp)
    target_link_libraries(imfwizard_job_queue_test PRIVATE imfwizard_core)
    add_test(NAME imfwizard_job_queue_test COMMAND imfwizard_job_queue_test)

    # CLI integration tests (Python)
    find_package(Python3 REQUIRED COMPONENTS Interpreter)
    add_test(NAME imfwizard_cli_test
        COMMAND ${Python3_EXECUTABLE} ${CMAKE_SOURCE_DIR}/tests/test_cli.py
        WORKING_DIRECTORY ${CMAKE_BINARY_DIR})

    # GUI launch/exit tests (Python, optional)
    add_test(NAME imfwizard_gui_test
        COMMAND ${Python3_EXECUTABLE} ${CMAKE_SOURCE_DIR}/tests/test_gui.py)

    # job_queue_test and cli_test both use /tmp/imfwizard.sock — serialize them
    set_tests_properties(imfwizard_job_queue_test imfwizard_cli_test
        PROPERTIES RESOURCE_LOCK "imfwizard_socket")
endif()

# Python bindings (SWIG)
if(BUILD_PYTHON_BINDINGS)
  add_subdirectory(bindings/python)
endif()
