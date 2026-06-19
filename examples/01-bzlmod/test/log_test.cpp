#include "logging/log.hpp"

#include <cstdint>

#include "logging/logger.hpp"
#include "gtest/gtest.h"

TEST(Log, TestMacro) {
  constexpr std::int16_t value = 42;
  LOG(Logger::INFO, "Logging with macro: {}", value);
}
