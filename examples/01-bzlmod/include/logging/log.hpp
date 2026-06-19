#pragma once

#include "logging/logger.hpp"

template <typename... Args>
constexpr void LOG(Logger::Level level, std::format_string<Args...> fmessage,
                   Args &&...args) {
  Logger::log(level, fmessage, std::forward<Args>(args)...);
}

template <typename... Args>
constexpr void LOG_INFO(std::format_string<Args...> fmessage, Args &&...args) {
  LOG(Logger::Level::INFO, fmessage, std::forward<Args>(args)...);
}

template <typename... Args>
constexpr void LOG_DEBUG(std::format_string<Args...> fmessage, Args &&...args) {
  LOG(Logger::Level::DEBUG, fmessage, std::forward<Args>(args)...);
}

template <typename... Args>
constexpr void LOG_WARN(std::format_string<Args...> fmessage, Args &&...args) {
  LOG(Logger::Level::WARN, fmessage, std::forward<Args>(args)...);
}

template <typename... Args>
constexpr void LOG_ERROR(std::format_string<Args...> fmessage, Args &&...args) {
  LOG(Logger::Level::ERROR, fmessage, std::forward<Args>(args)...);
}
