#include <cassert>
#include <chrono>
#include <cstdlib>
#include <filesystem>
#include <spdlog/spdlog.h>
#include <string>
#include <thread>

#include "imfwizard/job_queue.h"

#ifndef _WIN32
#include <signal.h>
#include <sys/wait.h>
#include <unistd.h>
#endif

namespace fs = std::filesystem;
using namespace imfwizard;

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

// --- Job type/state string conversion ---

void test_job_type_to_string()
{
  ASSERT(job_type_to_string(JobType::Transcode) == "transcode");
  ASSERT(job_type_to_string(JobType::Encode) == "encode");
  ASSERT(job_type_to_string(JobType::Create) == "create");
  ASSERT(job_type_to_string(JobType::Validate) == "validate");
  ASSERT(job_type_to_string(JobType::Loudness) == "loudness");
  ASSERT(job_type_to_string(JobType::QC) == "qc");
}

void test_job_type_from_string()
{
  ASSERT(job_type_from_string("transcode") == JobType::Transcode);
  ASSERT(job_type_from_string("encode") == JobType::Encode);
  ASSERT(job_type_from_string("create") == JobType::Create);
  ASSERT(job_type_from_string("validate") == JobType::Validate);
  ASSERT(job_type_from_string("loudness") == JobType::Loudness);
  ASSERT(job_type_from_string("qc") == JobType::QC);
}

void test_job_state_to_string()
{
  ASSERT(job_state_to_string(JobState::Queued) == "queued");
  ASSERT(job_state_to_string(JobState::Running) == "running");
  ASSERT(job_state_to_string(JobState::Completed) == "completed");
  ASSERT(job_state_to_string(JobState::Failed) == "failed");
  ASSERT(job_state_to_string(JobState::Cancelled) == "cancelled");
}

void test_job_state_from_string()
{
  ASSERT(job_state_from_string("queued") == JobState::Queued);
  ASSERT(job_state_from_string("running") == JobState::Running);
  ASSERT(job_state_from_string("completed") == JobState::Completed);
  ASSERT(job_state_from_string("failed") == JobState::Failed);
  ASSERT(job_state_from_string("cancelled") == JobState::Cancelled);
}

// --- Path tests ---

void test_jobs_file_path()
{
  auto p = jobs_file_path();
  ASSERT(!p.empty());
  // Should be under ~/.config/imfwizard/
  ASSERT(p.string().find("imfwizard") != std::string::npos);
  ASSERT(p.filename() == "jobs.json");
}

void test_socket_path()
{
  auto p = socket_path();
  ASSERT(!p.empty());
  ASSERT(p.string().find("imfwizard") != std::string::npos);
}

// --- Client when daemon is not running ---

void test_client_no_daemon()
{
  // Ensure no daemon is running by removing socket
  fs::remove(socket_path());
  JobQueueClient client;
  ASSERT(!client.is_daemon_running());
  auto jobs = client.list();
  ASSERT(jobs.empty());
  ASSERT(!client.cancel(999));
  ASSERT(client.submit(JobType::Transcode, "test", {}) == std::nullopt);
}

#ifndef _WIN32
// --- Daemon lifecycle (fork, communicate, kill) ---

static pid_t daemon_pid = -1;
static std::string test_socket;

void start_test_daemon()
{
  // Clean any lingering socket and jobs file
  fs::remove(socket_path());
  fs::remove(jobs_file_path());

  daemon_pid = fork();
  if(daemon_pid == 0)
  {
    // Child: run daemon
    JobQueueDaemon daemon;
    daemon.run();
    _exit(0);
  }
  // Parent: wait for daemon to start
  std::this_thread::sleep_for(std::chrono::milliseconds(500));
}

void stop_test_daemon()
{
  if(daemon_pid > 0)
  {
    kill(daemon_pid, SIGTERM);
    int status = 0;
    waitpid(daemon_pid, &status, 0);
    daemon_pid = -1;
  }
  // Clean socket
  fs::remove(socket_path());
}

void test_daemon_start_stop()
{
  start_test_daemon();
  JobQueueClient client;
  ASSERT(client.is_daemon_running());
  stop_test_daemon();
  // After stop, daemon should not be running
  std::this_thread::sleep_for(std::chrono::milliseconds(100));
  ASSERT(!client.is_daemon_running());
}

void test_submit_and_list()
{
  start_test_daemon();
  JobQueueClient client;

  auto id = client.submit(JobType::Encode, "Test encode job", {"--input", "/tmp/test"});
  ASSERT(id.has_value());
  ASSERT(*id >= 1);

  auto jobs = client.list();
  ASSERT(!jobs.empty());
  bool found = false;
  for(auto& j : jobs)
  {
    if(j.id == *id)
    {
      found = true;
      ASSERT(j.type == JobType::Encode);
      ASSERT(j.description == "Test encode job");
      ASSERT(j.state == JobState::Queued || j.state == JobState::Running);
    }
  }
  ASSERT(found);

  stop_test_daemon();
}

void test_cancel_job()
{
  start_test_daemon();
  JobQueueClient client;

  // Pause queue so jobs stay in Queued state
  client.pause();
  std::this_thread::sleep_for(std::chrono::milliseconds(100));

  auto id1 = client.submit(JobType::Transcode, "Cancel me", {"sleep", "60"});
  ASSERT(id1.has_value());

  std::this_thread::sleep_for(std::chrono::milliseconds(200));

  bool cancelled = client.cancel(*id1);
  ASSERT(cancelled);

  auto s = client.status(*id1);
  ASSERT(s.has_value());
  ASSERT(s->state == JobState::Cancelled);

  stop_test_daemon();
}

void test_priority()
{
  start_test_daemon();
  JobQueueClient client;

  // Pause so jobs stay queued
  client.pause();
  std::this_thread::sleep_for(std::chrono::milliseconds(100));

  auto id1 = client.submit(JobType::Encode, "Low priority", {}, std::nullopt, 0);
  auto id2 = client.submit(JobType::Encode, "High priority", {}, std::nullopt, 10);
  ASSERT(id1.has_value());
  ASSERT(id2.has_value());

  // Set priority on first job
  bool ok = client.set_priority(*id1, 20);
  ASSERT(ok);

  auto s = client.status(*id1);
  ASSERT(s.has_value());
  ASSERT(s->priority == 20);

  stop_test_daemon();
}

void test_pause_resume()
{
  start_test_daemon();
  JobQueueClient client;

  ASSERT(!client.is_paused());
  ASSERT(client.pause());
  ASSERT(client.is_paused());
  ASSERT(client.resume());
  ASSERT(!client.is_paused());

  stop_test_daemon();
}
#endif // _WIN32

int main()
{
  spdlog::info("Job Queue Tests");
  spdlog::info("===============");

  spdlog::info("Enums:");
  TEST(test_job_type_to_string);
  TEST(test_job_type_from_string);
  TEST(test_job_state_to_string);
  TEST(test_job_state_from_string);

  spdlog::info("Paths:");
  TEST(test_jobs_file_path);
  TEST(test_socket_path);

  spdlog::info("Client (no daemon):");
  TEST(test_client_no_daemon);

#ifndef _WIN32
  spdlog::info("Daemon lifecycle:");
  TEST(test_daemon_start_stop);
  TEST(test_submit_and_list);
  TEST(test_cancel_job);
  TEST(test_priority);
  TEST(test_pause_resume);
#endif

  spdlog::info("{}/{} tests passed", tests_passed, tests_run);
  return (tests_passed == tests_run) ? 0 : 1;
}
