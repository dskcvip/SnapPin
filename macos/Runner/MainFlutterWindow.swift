import Cocoa
import ApplicationServices
import FlutterMacOS
import desktop_multi_window
import Vision
import Security

class MainFlutterWindow: NSWindow {
  private var captureChannel: FlutterMethodChannel?
  private var ocrChannel: FlutterMethodChannel?
  private var ocrBusy = false
  private var escapeMonitor: Any?
  private var localEscapeMonitor: Any?
  private var requestedScreenPermission = false
  private var screenPermissionWarningAccepted = false

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
            let before = CGPreflightScreenCaptureAccess()
            var requestGranted = false
            if !before && !self.requestedScreenPermission {
              self.requestedScreenPermission = true
              requestGranted = CGRequestScreenCaptureAccess()
            }
            let after = CGPreflightScreenCaptureAccess()
            let details = Self.capturePermissionDetails()
            MacStartupLog.write("screen-recording-permission-check",
                                "before=\(before) requestGranted=\(requestGranted) after=\(after) details=\(details)")
            if !before && !requestGranted && !after && !self.screenPermissionWarningAccepted {
              // Match Tauri's mac_capture_gate: macOS 13 may report false even
              // when CGDisplayCreateImage can capture other applications. The OS
              // still enforces access; do not treat preflight as a hard gate.
              let alert = NSAlert()
              alert.messageText = "屏幕录制权限检查"
              alert.informativeText = "系统报告 SnapPin 没有‘屏幕录制’权限，但 macOS 13 的检测可能误报。\n\n可以继续截图。如果截图能看到其他窗口，忽略此提示即可，本次运行不再提醒。若只有桌面壁纸，请从托盘或悬浮球菜单打开‘权限检查…’，重置旧授权并按提示重新授权。"
              alert.addButton(withTitle: "继续截图")
              alert.addButton(withTitle: "权限检查…")
              alert.addButton(withTitle: "取消")
              let choice = alert.runModal()
              MacStartupLog.write("screen-recording-warning-action", "response=\(choice.rawValue)")
              if choice == .alertFirstButtonReturn {
                self.screenPermissionWarningAccepted = true
                MacStartupLog.write("screen-recording-preflight-override", "user=continue; OS capture restrictions remain active")
              } else if choice == .alertSecondButtonReturn {
                self.checkAndFixPermissions { _ in
                  result(FlutterError(code: "capture_cancelled", message: "已打开权限检查，未开始截图", details: details))
                }
                return
              } else {
                result(FlutterError(code: "capture_cancelled", message: "已取消截图", details: details))
                return
              }
            }
          }
          DispatchQueue.global(qos: .userInitiated).async {
            do {
              let displays = try Self.captureDisplays()
              MacStartupLog.write("capture-displays-native-ready", "count=\(displays.count); image creation does not prove other-window access")
              DispatchQueue.main.async { result(displays) }
            } catch {
              DispatchQueue.main.async {
                result(FlutterError(code: "capture_failed", message: error.localizedDescription, details: nil))
              }
            }
          }
        case "capturePermissionStatus":
          result(Self.capturePermissionDetails())
        case "checkAndFixPermissions":
          self.checkAndFixPermissions(result: result)
        case "showCapturePermissionHelp":
          let alert = NSAlert()
          alert.messageText = "需要屏幕录制权限"
          let appPath = Bundle.main.bundleURL.path
          let temporaryLaunch = appPath.hasPrefix("/Volumes/") || appPath.contains("/AppTranslocation/")
          alert.informativeText = (temporaryLaunch
            ? "当前正在安装镜像或临时路径中运行。请先退出，将 SnapPin 拖到‘应用程序’，推出安装镜像，再从‘应用程序’打开。\n\n"
            : "") + "请在系统设置 → 隐私与安全性 → 屏幕录制中允许 SnapPin。\n\n如果已经开启仍提示，请选中旧的 SnapPin 条目，点减号移除，再点加号添加‘应用程序’中的 SnapPin。完成后从托盘菜单退出，再重新打开；关闭窗口不会退出程序。"
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

  /// Include the actual running bundle and signing requirement in both logs.
  /// A checked TCC entry for an older build is not proof this process is trusted.
  private static func capturePermissionDetails() -> [String: Any] {
    var details: [String: Any] = [
      "bundleId": Bundle.main.bundleIdentifier ?? "",
      "bundlePath": Bundle.main.bundleURL.path,
      "executablePath": Bundle.main.executableURL?.path ?? "",
      "pid": Int(ProcessInfo.processInfo.processIdentifier),
      "os": ProcessInfo.processInfo.operatingSystemVersionString,
      "accessibilityGranted": AXIsProcessTrusted(),
    ]
    if #available(macOS 10.15, *) {
      details["preflightGranted"] = CGPreflightScreenCaptureAccess()
    }
    var code: SecCode?
    let codeStatus = SecCodeCopySelf([], &code)
    details["signatureStatus"] = Int(codeStatus)
    if let code = code {
      var requirement: SecRequirement?
      let status = SecCodeCopyDesignatedRequirement(code, [], &requirement)
      details["requirementStatus"] = Int(status)
      if let requirement = requirement {
        var text: CFString?
        if SecRequirementCopyString(requirement, [], &text) == errSecSuccess {
          details["designatedRequirement"] = text as String? ?? ""
        }
      }
    }
    return details
  }

  private func checkAndFixPermissions(result: @escaping FlutterResult) {
    let details = Self.capturePermissionDetails()
    MacStartupLog.write("permission-check", "\(details)")
    let screen = details["preflightGranted"] as? Bool ?? true
    let accessibility = details["accessibilityGranted"] as? Bool ?? false
    let mark: (Bool) -> String = { $0 ? "✓ 已授权" : "✗ 系统报告未授权" }
    let alert = NSAlert()
    alert.messageText = screen && accessibility ? "权限检查：全部就绪" : "权限检查"
    alert.informativeText = "屏幕录制：\(mark(screen))\n（截图用；macOS 13 可能误报，可尝试继续截图）\n\n辅助功能：\(mark(accessibility))\n（长截图自动滚动用）\n\n当前为临时签名版本。升级后旧授权可能‘勾着但无效’，可用‘重置授权’清除本应用的两项旧记录，再重新授权。\n\n请将 SnapPin 放入‘应用程序’后运行，不要直接从安装镜像打开。"
    if screen && accessibility {
      alert.addButton(withTitle: "好")
      alert.runModal()
      result(["action": "ready", "status": details])
      return
    }
    alert.addButton(withTitle: "去系统设置")
    alert.addButton(withTitle: "重置授权")
    alert.addButton(withTitle: "取消")
    switch alert.runModal() {
    case .alertFirstButtonReturn:
      let pane = screen ? "Privacy_Accessibility" : "Privacy_ScreenCapture"
      let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?\(pane)")!
      let opened = NSWorkspace.shared.open(url)
      MacStartupLog.write("permission-settings-open", "pane=\(pane) opened=\(opened)")
      result(["action": "settings", "opened": opened, "status": details])
    case .alertSecondButtonReturn:
      // Only an explicit click resets TCC, only for this running bundle ID.
      // Do not reset all applications or change privacy database files.
      guard let identifier = Bundle.main.bundleIdentifier, !identifier.isEmpty else {
        result(FlutterError(code: "permission_reset_failed", message: "无法读取应用标识，未重置授权", details: details))
        return
      }
      MacStartupLog.write("permission-reset-start", "bundleId=\(identifier)")
      DispatchQueue.global(qos: .userInitiated).async {
        var outcomes: [String: Any] = [:]
        var succeeded = true
        for service in ["ScreenCapture", "Accessibility"] {
          let process = Process()
          process.executableURL = URL(fileURLWithPath: "/usr/bin/tccutil")
          process.arguments = ["reset", service, identifier]
          let output = Pipe()
          process.standardOutput = output
          process.standardError = output
          do {
            try process.run()
            let data = output.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            let message = String(data: data, encoding: .utf8) ?? ""
            outcomes[service] = ["exitCode": Int(process.terminationStatus), "output": message]
            if process.terminationStatus != 0 { succeeded = false }
          } catch {
            succeeded = false
            outcomes[service] = ["error": error.localizedDescription]
          }
        }
        let resetResults = outcomes
        let resetSucceeded = succeeded
        DispatchQueue.main.async {
          MacStartupLog.write("permission-reset-finished", "bundleId=\(identifier) results=\(resetResults)")
          let notice = NSAlert()
          notice.messageText = resetSucceeded ? "已重置权限" : "部分授权重置失败"
          notice.informativeText = (resetSucceeded ? "本应用的屏幕录制和辅助功能旧授权记录都已清除。" : "请在终端执行：\ntccutil reset ScreenCapture \(identifier)\ntccutil reset Accessibility \(identifier)") + "\n\n接下来：\n① 从托盘菜单‘退出’完全退出 SnapPin；\n② 从‘应用程序’重新打开 SnapPin，按系统提示允许屏幕录制；\n③ 到系统设置 → 隐私与安全性 → 辅助功能，添加并勾选 SnapPin；\n④ 系统要求重新打开应用时，再退出重开一次。"
          notice.addButton(withTitle: "好")
          notice.runModal()
          result(["action": "reset", "success": resetSucceeded, "results": resetResults, "status": details])
        }
      }
    default:
      MacStartupLog.write("permission-check-cancelled")
      result(["action": "cancel", "status": details])
    }
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
