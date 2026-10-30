#ifndef RUNNER_OCR_BRIDGE_H_
#define RUNNER_OCR_BRIDGE_H_

#include <cstdint>
#include <string>
#include <vector>

std::string RecognizeText(const std::vector<uint8_t>& image_bytes);

#endif  // RUNNER_OCR_BRIDGE_H_
