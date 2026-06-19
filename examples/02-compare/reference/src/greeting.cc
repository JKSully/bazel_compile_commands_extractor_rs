#include "example/greeting.h"

#include <utility>

namespace example {

std::string greeting(std::string name) { return "hello, " + std::move(name); }

} // namespace example
