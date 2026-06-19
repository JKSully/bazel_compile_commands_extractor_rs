#pragma once

#include <cstdint>
#include <format>
#include <string_view>
#include <utility>

class Logger {
public:
  enum Level : std::uint8_t { INFO, DEBUG, WARN, ERROR };

  ~Logger() = default;

  Logger &operator=(const Logger &) = delete;
  Logger(const Logger &) = delete;

  Logger &operator=(Logger &&other) = default;
  Logger(Logger &&other) = default;

  template <typename... Args>
  static void log(Level level, std::format_string<Args...> fmessage,
                  Args &&...args) {
    return log(level, std::format(fmessage, std::forward<Args>(args)...));
  }

  template <typename... Args>
  static void log(Level level, std::string_view fmessage, Args &&...args) {
    return log(level, std::vformat(fmessage, std::forward<Args>(args)...));
  }

  static void log(Level level, std::string msg);

private:
  Logger() = default;
};
