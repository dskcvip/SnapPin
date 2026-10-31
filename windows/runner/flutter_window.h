#ifndef RUNNER_FLUTTER_WINDOW_H_
#define RUNNER_FLUTTER_WINDOW_H_

#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <flutter/method_channel.h>
#include <flutter/encodable_value.h>

#include <memory>
#include <vector>

#include "win32_window.h"

struct OcrAsyncState;

// A window that does nothing but host a Flutter view.
class FlutterWindow : public Win32Window {
 public:
  // Creates a new FlutterWindow hosting a Flutter view running |project|.
  explicit FlutterWindow(const flutter::DartProject& project);
  virtual ~FlutterWindow();

 protected:
  // Win32Window:
  bool OnCreate() override;
  void OnDestroy() override;
  LRESULT MessageHandler(HWND window, UINT const message, WPARAM const wparam,
                         LPARAM const lparam) noexcept override;

 private:
  // The project to run.
  flutter::DartProject project_;

  // The Flutter instance hosted by this window.
  std::unique_ptr<flutter::FlutterViewController> flutter_controller_;
  std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>> capture_channel_;
  std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>> ocr_channel_;
  std::shared_ptr<OcrAsyncState> ocr_state_;
  HWND tooltip_ = nullptr;
  bool long_escape_registered_ = false;
  bool suppress_input_toolbar_ = false;
  std::vector<HWND> hidden_input_toolbars_;
};

#endif  // RUNNER_FLUTTER_WINDOW_H_
