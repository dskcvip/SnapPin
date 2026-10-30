#include "ocr_bridge.h"

#include <future>
#include <stdexcept>

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Globalization.h>
#include <winrt/Windows.Graphics.Imaging.h>
#include <winrt/Windows.Media.Ocr.h>
#include <winrt/Windows.Storage.Streams.h>
#include <winrt/base.h>

std::string RecognizeText(const std::vector<uint8_t>& image_bytes) {
  if (image_bytes.empty()) throw std::runtime_error("Empty OCR image");
  return std::async(std::launch::async, [&image_bytes]() {
    winrt::init_apartment(winrt::apartment_type::multi_threaded);
    using namespace winrt::Windows::Storage::Streams;
    using namespace winrt::Windows::Graphics::Imaging;
    using namespace winrt::Windows::Media::Ocr;

    InMemoryRandomAccessStream stream;
    DataWriter writer(stream);
    writer.WriteBytes(winrt::array_view<const uint8_t>(
        image_bytes.data(), image_bytes.data() + image_bytes.size()));
    writer.StoreAsync().get();
    writer.DetachStream();
    stream.Seek(0);

    auto decoder = BitmapDecoder::CreateAsync(stream).get();
    auto bitmap = decoder.GetSoftwareBitmapAsync().get();
    OcrEngine engine{nullptr};
    for (const auto& language : OcrEngine::AvailableRecognizerLanguages()) {
      const auto tag = winrt::to_string(language.LanguageTag());
      if (tag.rfind("zh", 0) == 0) {
        engine = OcrEngine::TryCreateFromLanguage(language);
        if (engine) break;
      }
    }
    if (!engine) engine = OcrEngine::TryCreateFromUserProfileLanguages();
    if (!engine) throw std::runtime_error("No local OCR language is installed");
    auto result = engine.RecognizeAsync(bitmap).get();
    std::string text;
    for (const auto& line : result.Lines()) {
      if (!text.empty()) text += "\n";
      text += winrt::to_string(line.Text());
    }
    return text;
  }).get();
}
