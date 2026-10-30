#ifndef RUNNER_CAPTURE_BRIDGE_H_
#define RUNNER_CAPTURE_BRIDGE_H_

#include <flutter/encodable_value.h>
#include <cstdint>
#include <vector>

flutter::EncodableList CaptureDisplays();
std::vector<uint8_t> CaptureRegion(int x, int y, int width, int height);

#endif  // RUNNER_CAPTURE_BRIDGE_H_
