#ifndef RUNNER_OCR_BRIDGE_H_
#define RUNNER_OCR_BRIDGE_H_

#include <cstdint>
#include <string>
#include <vector>

struct OcrTextLine {
  std::string text;
  double x, y, width, height;
};
struct OcrLayout {
  uint32_t width = 0, height = 0;
  std::vector<OcrTextLine> lines;
};
std::string RecognizeText(const std::vector<uint8_t>& image_bytes,
                          OcrLayout* layout = nullptr);

#endif  // RUNNER_OCR_BRIDGE_H_
