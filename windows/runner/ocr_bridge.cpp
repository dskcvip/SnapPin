#include "ocr_bridge.h"

#include <future>
#include <algorithm>
#include <cmath>
#include <cwctype>
#include <stdexcept>

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Globalization.h>
#include <winrt/Windows.Graphics.Imaging.h>
#include <winrt/Windows.Media.Ocr.h>
#include <winrt/Windows.Storage.Streams.h>
#include <winrt/base.h>

std::string RecognizeText(const std::vector<uint8_t>& image_bytes, OcrLayout* layout) {
  if (image_bytes.empty()) throw std::runtime_error("Empty OCR image");
  try {
    return std::async(std::launch::async, [&image_bytes, layout]() {
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
    // Enlarge small screen fonts for recognition only. Decode overlapping strips
    // so tall screenshots keep their detail without exceeding the OS OCR limit.
    const auto width = decoder.PixelWidth();
    const auto height = decoder.PixelHeight();
    if (layout) { layout->width = width; layout->height = height; layout->lines.clear(); }
    const auto limit = OcrEngine::MaxImageDimension();
    const double scale = std::min(2.5, static_cast<double>(limit) / width);
    const auto scaled_width = static_cast<uint32_t>(std::floor(width * scale));
    const auto scaled_height = static_cast<uint32_t>(std::ceil(height * scale));
    const uint32_t overlap = std::min<uint32_t>(64, limit / 4);
    const uint32_t stride = limit - overlap;
    std::string text;
    for (uint32_t y = 0; y < scaled_height; y += stride) {
      const uint32_t tile_height = std::min(limit, scaled_height - y);
      BitmapTransform transform;
      transform.ScaledWidth(scaled_width);
      transform.ScaledHeight(scaled_height);
      transform.InterpolationMode(BitmapInterpolationMode::Cubic);
      transform.Bounds(BitmapBounds{0, y, scaled_width, tile_height});
      auto bitmap = decoder.GetSoftwareBitmapAsync(
          BitmapPixelFormat::Bgra8, BitmapAlphaMode::Ignore, transform,
          ExifOrientationMode::IgnoreExifOrientation,
          ColorManagementMode::DoNotColorManage).get();
      auto result = engine.RecognizeAsync(bitmap).get();
      for (const auto& line : result.Lines()) {
        double top = tile_height;
        double bottom = 0;
        double left = scaled_width;
        double right = 0;
        for (const auto& word : line.Words()) {
          const auto bounds = word.BoundingRect();
          top = std::min(top, static_cast<double>(bounds.Y));
          bottom = std::max(bottom, static_cast<double>(bounds.Y + bounds.Height));
          left = std::min(left, static_cast<double>(bounds.X));
          right = std::max(right, static_cast<double>(bounds.X + bounds.Width));
        }
        const auto middle = (top + bottom) / 2;
        if (y > 0 && middle < overlap / 2.0) continue;
        if (y + tile_height < scaled_height && middle >= tile_height - overlap / 2.0) continue;
        // Windows inserts spaces between CJK characters. Remove only those
        // separators; preserve spaces in Latin text, numbers and punctuation.
        const auto raw = line.Text();
        std::wstring cleaned;
        const auto is_cjk = [](wchar_t ch) {
          return (ch >= 0x3400 && ch <= 0x9fff) ||
                 (ch >= 0xf900 && ch <= 0xfaff);
        };
        for (uint32_t i = 0; i < raw.size(); ++i) {
          if (iswspace(raw[i]) && !cleaned.empty() && is_cjk(cleaned.back())) {
            uint32_t next = i + 1;
            while (next < raw.size() && iswspace(raw[next])) ++next;
            if (next < raw.size() && is_cjk(raw[next])) continue;
          }
          cleaned += raw[i];
        }
        if (!text.empty()) text += "\n";
        text += winrt::to_string(cleaned);
        if (layout && right > left && bottom > top) {
          layout->lines.push_back({winrt::to_string(cleaned), left / scale,
            (y + top) / scale, (right - left) / scale, (bottom - top) / scale});
        }
      }
      if (y + tile_height == scaled_height) break;
    }
    return text;
    }).get();
  } catch (const winrt::hresult_error& error) {
    // Surface WinRT errors through the existing Flutter error channel.
    throw std::runtime_error(winrt::to_string(error.message()));
  }
}
