import Cocoa
import ApplicationServices
import FlutterMacOS
import desktop_multi_window
import Vision

class MainFlutterWindow: NSWindow {
  private var captureChannel: FlutterMethodChannel?
  private var ocrChannel: FlutterMethodChannel?
  private var ocrBusy = false
  private var escapeMonitor: Any?
  private var localEscapeMonitor: Any?

  override func awakeFromNib() {
    MacStartupLog.write("native-window-awake")
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    RegisterGeneratedPlugins(registry: flutterViewController)
    MacStartupLog.write("native-plugins-registered")
    FlutterMultiWindowPlugin.setOnWindowCreatedCallback { controller in
      MacStartupLog.write("native-child-created")
      RegisterGeneratedPlugins(registry: controller)
      let channel = FlutterMethodChannel(
        name: "snappin/capture", binaryMessenger: controller.engine.binaryMessenger)
      channel.setMethodCallHandler { [weak controller] call, result in
        guard call.method == "windowSnap",
              let args = call.arguments as? [String: Any] else {
          result(FlutterMethodNotImplemented)
          return
        }
        result(Self.windowSnap(args, excluding: controller?.view.window?.windowNumber))
      }
      // Hidden prewarmed windows may never receive viewWillAppear on macOS 13.
      // FlutterViewController normally starts its Dart engine from that callback.
      // Explicitly start it so the child can register its window channel while hidden.
      // FlutterEngine documents repeated run calls as safe no-ops.
      let started = controller.engine.run(withEntrypoint: nil)
      MacStartupLog.write("native-child-engine-run", "success=\(started)")
    }

    captureChannel = FlutterMethodChannel(
      name: "snappin/capture",
      binaryMessenger: flutterViewController.engine.binaryMessenger
    )
    captureChannel?.setMethodCallHandler { call, result in
      do {
        switch call.method {
        case "captureDisplays":
          if #available(macOS 10.15, *) {
            guard CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess() else {
              MacStartupLog.write("screen-recording-permission-denied")
              result(FlutterError(code: "screen_recording_permission_required",
                                  message: "需要屏幕录制权限", details: nil))
              return
            }
          }
          DispatchQueue.global(qos: .userInitiated).async {
            do {
              let displays = try Self.captureDisplays()
              DispatchQueue.main.async { result(displays) }
            } catch {
              DispatchQueue.main.async {
                result(FlutterError(code: "capture_failed", message: error.localizedDescription, details: nil))
              }
            }
          }
        case "showCapturePermissionHelp":
          let alert = NSAlert()
          alert.messageText = "需要屏幕录制权限"
          alert.informativeText = "请在系统设置 → 隐私与安全性 → 屏幕录制中允许 SnapPin。授权后，请退出并重新打开 SnapPin，再开始截图。"
          alert.addButton(withTitle: "打开系统设置")
          alert.addButton(withTitle: "取消")
          let openSettings = alert.runModal() == .alertFirstButtonReturn
          if openSettings, let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture") {
            NSWorkspace.shared.open(url)
          }
          MacStartupLog.write("screen-recording-permission-help", "openSettings=\(openSettings)")
          result(openSettings)
        case "windowSnap":
          guard let args = call.arguments as? [String: Any] else {
            result(nil)
            return
          }
          result(Self.windowSnap(args, excluding: self.windowNumber))
        case "captureRegion":
          guard let args = call.arguments as? [String: Int],
                let x = args["x"], let y = args["y"],
                let width = args["width"], let height = args["height"] else {
            throw NSError(domain: "SnapPin", code: 5,
                          userInfo: [NSLocalizedDescriptionKey: "Invalid capture region"])
          }
          DispatchQueue.global(qos: .userInitiated).async {
            do {
              let image = try Self.captureRegion(x: x, y: y, width: width, height: height)
              DispatchQueue.main.async { result(FlutterStandardTypedData(bytes: image)) }
            } catch {
              DispatchQueue.main.async {
                result(FlutterError(code: "capture_failed", message: error.localizedDescription, details: nil))
              }
            }
          }
        case "suppressInputToolbar":
          // The Windows Sogou status window has no macOS counterpart.
          result(0)
        case "scrollAt":
          guard let args = call.arguments as? [String: Any],
                let x = args["x"] as? Int, let y = args["y"] as? Int,
                let lines = args["lines"] as? Int else {
            throw NSError(domain: "SnapPin", code: 6,
                          userInfo: [NSLocalizedDescriptionKey: "Invalid scroll region"])
          }
          guard AXIsProcessTrusted() else {
            throw NSError(domain: "SnapPin", code: 7,
                          userInfo: [NSLocalizedDescriptionKey: "Accessibility permission is required for long capture"])
          }
          let wheel = CGEvent(scrollWheelEvent2Source: nil, units: .line,
                              wheelCount: 1, wheel1: Int32(-lines),
                              wheel2: 0, wheel3: 0)
          wheel?.location = CGPoint(
            x: args["logicalX"] as? Double ?? Double(x),
            y: args["logicalY"] as? Double ?? Double(y))
          wheel?.post(tap: .cghidEventTap)
          result(nil)
        case "setLongEscape":
          let enabled = call.arguments as? Bool ?? false
          if enabled {
            if self.escapeMonitor == nil {
              self.escapeMonitor = NSEvent.addGlobalMonitorForEvents(matching: .keyDown) { [weak self] event in
                if event.keyCode == 53 {
                  self?.captureChannel?.invokeMethod("longEscape", arguments: nil)
                }
              }
            }
            if self.localEscapeMonitor == nil {
              self.localEscapeMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                if event.keyCode == 53 {
                  self?.captureChannel?.invokeMethod("longEscape", arguments: nil)
                  return nil
                }
                return event
              }
            }
          } else {
            if let monitor = self.escapeMonitor {
              NSEvent.removeMonitor(monitor)
              self.escapeMonitor = nil
            }
            if let localMonitor = self.localEscapeMonitor {
              NSEvent.removeMonitor(localMonitor)
              self.localEscapeMonitor = nil
            }
          }
          result(nil)
        default:
          result(FlutterMethodNotImplemented)
        }
      } catch {
        result(FlutterError(code: "capture_failed", message: error.localizedDescription, details: nil))
      }
    }

    ocrChannel = FlutterMethodChannel(
      name: "snappin/ocr",
      binaryMessenger: flutterViewController.engine.binaryMessenger
    )
    ocrChannel?.setMethodCallHandler { call, result in
      guard call.method == "recognizeText" || call.method == "recognizeLayout" else {
        result(FlutterMethodNotImplemented)
        return
      }
      guard let bytes = (call.arguments as? FlutterStandardTypedData)?.data else {
        result(FlutterError(code: "ocr_failed", message: "Invalid image data", details: nil))
        return
      }
      let wantsLayout = call.method == "recognizeLayout"
      guard !self.ocrBusy else {
        result(FlutterError(code: "ocr_busy", message: "文字识别正在进行，请稍后再试", details: nil))
        return
      }
      self.ocrBusy = true
      DispatchQueue.global(qos: .userInitiated).async {
        do {
          let request = VNRecognizeTextRequest()
          request.recognitionLevel = .accurate
          request.usesLanguageCorrection = true
          let supported = try request.supportedRecognitionLanguages()
          let preferred = ["zh-Hans", "en-US"].filter { supported.contains($0) }
          if !preferred.isEmpty {
            request.recognitionLanguages = preferred
          }
          if #available(macOS 13.0, *) {
            request.automaticallyDetectsLanguage = true
          }
          let handler = VNImageRequestHandler(data: bytes, options: [:])
          try handler.perform([request])
          let observations = (request.results ?? []).sorted {
            $0.boundingBox.midY > $1.boundingBox.midY
          }
          let text = observations.compactMap { $0.topCandidates(1).first?.string }
            .joined(separator: "\n")
          if wantsLayout {
            guard let image = NSImage(data: bytes),
                  let representation = image.representations.first else {
              throw NSError(domain: "SnapPinOCR", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "无法读取原图尺寸"])
            }
            let width = Double(representation.pixelsWide)
            let height = Double(representation.pixelsHigh)
            let lines: [[String: Any]] = observations.compactMap { observation in
              guard let candidate = observation.topCandidates(1).first else { return nil }
              let bounds = observation.boundingBox
              return ["text": candidate.string, "x": Double(bounds.minX) * width,
                "y": (1 - Double(bounds.maxY)) * height,
                "width": Double(bounds.width) * width, "height": Double(bounds.height) * height]
            }
            DispatchQueue.main.async {
              self.ocrBusy = false
              result(["text": text, "width": width, "height": height, "lines": lines])
            }
          } else {
            DispatchQueue.main.async { self.ocrBusy = false; result(text) }
          }
        } catch {
          DispatchQueue.main.async {
            self.ocrBusy = false
            result(FlutterError(code: "ocr_failed", message: error.localizedDescription, details: nil))
          }
        }
      }
    }

    super.awakeFromNib()
    MacStartupLog.write("native-window-ready")
  }

  private static func windowSnap(_ args: [String: Any], excluding number: Int?) -> [String: Double]? {
    guard let x = args["x"] as? Double, let y = args["y"] as? Double,
          let displayX = args["displayX"] as? Double,
          let displayY = args["displayY"] as? Double,
          let displayWidth = args["displayWidth"] as? Int,
          let displayHeight = args["displayHeight"] as? Int else { return nil }
    guard let windows = CGWindowListCopyWindowInfo(
      [.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
      as? [[String: Any]] else { return nil }
    for window in windows {
      guard (window[kCGWindowLayer as String] as? Int) == 0,
            (window[kCGWindowNumber as String] as? Int) != number,
            let bounds = window[kCGWindowBounds as String] as? [String: Any],
            let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary),
            rect.width >= 12, rect.height >= 12,
            rect.contains(CGPoint(x: x, y: y)) else { continue }
      let left = max(0, rect.minX - displayX)
      let top = max(0, rect.minY - displayY)
      let right = min(Double(displayWidth), rect.maxX - displayX)
      let bottom = min(Double(displayHeight), rect.maxY - displayY)
      if right <= left || bottom <= top { return nil }
      return ["x": left, "y": top, "width": right - left, "height": bottom - top]
    }
    return nil
  }

  private static func captureRegion(x: Int, y: Int, width: Int, height: Int) throws -> Data {
    guard width > 0 && height > 0 else {
      throw NSError(domain: "SnapPin", code: 8,
                    userInfo: [NSLocalizedDescriptionKey: "Invalid region size"])
    }
    var count: UInt32 = 0
    guard CGGetActiveDisplayList(0, nil, &count) == .success else {
      throw NSError(domain: "SnapPin", code: 9,
                    userInfo: [NSLocalizedDescriptionKey: "Cannot enumerate displays"])
    }
    var ids = [CGDirectDisplayID](repeating: 0, count: Int(count))
    guard CGGetActiveDisplayList(count, &ids, &count) == .success else {
      throw NSError(domain: "SnapPin", code: 10,
                    userInfo: [NSLocalizedDescriptionKey: "Cannot read displays"])
    }
    for display in ids.prefix(Int(count)) {
      let bounds = CGDisplayBounds(display)
      let displayRect = bounds
      let region = CGRect(x: x, y: y, width: width, height: height)
      if displayRect.contains(region) {
        // Capture in native pixels, then convert the logical selection explicitly.
        // Passing a point-space rect into display image APIs is ambiguous on Retina.
        guard let fullImage = CGDisplayCreateImage(display) else { continue }
        let scaleX = Double(fullImage.width) / Double(bounds.width)
        let scaleY = Double(fullImage.height) / Double(bounds.height)
        let left = ((Double(x) - Double(bounds.minX)) * scaleX).rounded()
        let top = ((Double(y) - Double(bounds.minY)) * scaleY).rounded()
        let right = ((Double(x + width) - Double(bounds.minX)) * scaleX).rounded()
        let bottom = ((Double(y + height) - Double(bounds.minY)) * scaleY).rounded()
        let local = CGRect(x: left, y: top, width: right - left, height: bottom - top)
        if let image = fullImage.cropping(to: local),
           let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) {
          return png
        }
      }
    }
    throw NSError(domain: "SnapPin", code: 11,
                  userInfo: [NSLocalizedDescriptionKey: "Cannot capture selected region"])
  }

  private static func captureDisplays() throws -> [[String: Any]] {
    var count: UInt32 = 0
    guard CGGetActiveDisplayList(0, nil, &count) == .success else {
      throw NSError(domain: "SnapPin", code: 2,
                    userInfo: [NSLocalizedDescriptionKey: "Cannot enumerate displays"])
    }
    var ids = [CGDirectDisplayID](repeating: 0, count: Int(count))
    guard CGGetActiveDisplayList(count, &ids, &count) == .success else {
      throw NSError(domain: "SnapPin", code: 3,
                    userInfo: [NSLocalizedDescriptionKey: "Cannot read displays"])
    }

    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("snappin-flutter")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    return try ids.prefix(Int(count)).map { display in
      guard let image = CGDisplayCreateImage(display),
            let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else {
        throw NSError(domain: "SnapPin", code: 4,
                      userInfo: [NSLocalizedDescriptionKey: "Cannot capture display"])
      }
      let path = directory.appendingPathComponent("capture-\(UUID().uuidString).png")
      try png.write(to: path)
      let bounds = CGDisplayBounds(display)
      let pixelWidth = image.width
      let pixelHeight = image.height
      let scale = Double(pixelWidth) / Double(bounds.width)
      return [
        "path": path.path,
        "x": Double(bounds.origin.x),
        "y": Double(bounds.origin.y),
        "width": Double(bounds.width),
        "height": Double(bounds.height),
        "pixelWidth": Int(pixelWidth),
        "pixelHeight": Int(pixelHeight),
        "pixelX": Int((Double(bounds.origin.x) * scale).rounded()),
        "pixelY": Int((Double(bounds.origin.y) * scale).rounded()),
        "scale": scale,
      ]
    }
  }
}
