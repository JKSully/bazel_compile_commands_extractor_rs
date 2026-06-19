#include "logging/logger.hpp"

#include <array>
#include <print>
#include <string>

void Logger::log(Logger::Level level, std::string msg) {
  static constexpr std::array PREFIX = {
      "\033[32m[INFO]\033[0m ", "\033[36m[DEBUG]\033[0m ",
      "\033[33m[WARN]\033[0m ", "\033[31m[ERROR]\033[0m "};
  std::println("{}{}", PREFIX.at(level), msg);
}
