#import <Foundation/Foundation.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <CoreVideo/CoreVideo.h>
#include <dlfcn.h>
#include <stdio.h>
#include <unistd.h>

typedef void (*SPFrameCallback)(void *, const uint8_t *, size_t, size_t, size_t, double);

API_AVAILABLE(macos(12.3))
@interface SPLongStream : NSObject <SCStreamOutput, SCStreamDelegate>
@property(nonatomic, strong) SCStream *stream;
@property(nonatomic, strong) dispatch_queue_t outputQueue;
@property(nonatomic) SPFrameCallback callback;
@property(nonatomic) void *context;
@property(atomic) BOOL stopped;
@end

@implementation SPLongStream
- (void)stream:(SCStream *)stream didOutputSampleBuffer:(CMSampleBufferRef)sample
        ofType:(SCStreamOutputType)type {
    if (self.stopped || type != SCStreamOutputTypeScreen || !CMSampleBufferIsValid(sample)) return;
    // Idle/blank frames must not enter the stitch queue.
    static NSString *statusKey;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        NSString *const *key = dlsym(RTLD_DEFAULT, "SCStreamFrameInfoStatus");
        if (key) statusKey = *key;
    });
    CFArrayRef attachments = CMSampleBufferGetSampleAttachmentsArray(sample, false);
    if (statusKey && attachments && CFArrayGetCount(attachments)) {
        NSDictionary *info = (__bridge NSDictionary *)CFArrayGetValueAtIndex(attachments, 0);
        NSNumber *status = info[statusKey];
        if (status && status.integerValue != SCFrameStatusComplete) return;
    }
    CVPixelBufferRef buffer = CMSampleBufferGetImageBuffer(sample);
    if (!buffer || CVPixelBufferIsPlanar(buffer)) return;
    if (CVPixelBufferLockBaseAddress(buffer, kCVPixelBufferLock_ReadOnly) != kCVReturnSuccess) return;
    if (!self.stopped && self.callback) {
        self.callback(self.context, CVPixelBufferGetBaseAddress(buffer),
                      CVPixelBufferGetWidth(buffer), CVPixelBufferGetHeight(buffer),
                      CVPixelBufferGetBytesPerRow(buffer),
                      CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sample)));
    }
    CVPixelBufferUnlockBaseAddress(buffer, kCVPixelBufferLock_ReadOnly);
}

- (void)stream:(SCStream *)stream didStopWithError:(NSError *)error {
    fprintf(stderr, "[snappin] mac stream stopped: %s\n", error.localizedDescription.UTF8String);
    // Deliver errors on the same serial queue as frames; Rust owns the context there.
    dispatch_async(self.outputQueue, ^{
        if (!self.stopped && self.callback) self.callback(self.context, NULL, 0, 0, 0, 0);
    });
}
@end

static void SPError(char *out, size_t size, NSString *message) {
    if (out && size) snprintf(out, size, "%s", message.UTF8String ?: "unknown stream error");
}

static void SPStop(SPLongStream *session) API_AVAILABLE(macos(12.3));
static void SPStop(SPLongStream *session) {
    session.stopped = YES;
    NSError *removeError = nil;
    [session.stream removeStreamOutput:session type:SCStreamOutputTypeScreen error:&removeError];
    // Drain callbacks before the Rust context is released, even if stopCapture times out.
    dispatch_sync(session.outputQueue, ^{
        session.callback = NULL;
        session.context = NULL;
    });
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    [session.stream stopCaptureWithCompletionHandler:^(NSError *error) {
        dispatch_semaphore_signal(done);
    }];
    dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC));
    session.stream = nil;
}

void *snappin_long_stream_start(uint32_t displayID, double x, double y, double width, double height,
                               uint32_t pixelWidth, uint32_t pixelHeight,
                               SPFrameCallback callback, void *context, char *error, size_t errorSize) {
    @autoreleasepool {
        if (@available(macOS 12.3, *)) {
            // No load-time dependency: older macOS releases still launch normally.
            static void *framework;
            static dispatch_once_t once;
            dispatch_once(&once, ^{
                framework = dlopen("/System/Library/Frameworks/ScreenCaptureKit.framework/ScreenCaptureKit",
                                   RTLD_NOW | RTLD_GLOBAL);
            });
            if (!framework) { SPError(error, errorSize, @"ScreenCaptureKit unavailable"); return NULL; }
            Class contentClass = NSClassFromString(@"SCShareableContent");
            Class filterClass = NSClassFromString(@"SCContentFilter");
            Class configClass = NSClassFromString(@"SCStreamConfiguration");
            Class streamClass = NSClassFromString(@"SCStream");
            if (!contentClass || !filterClass || !configClass || !streamClass) {
                SPError(error, errorSize, @"ScreenCaptureKit classes unavailable"); return NULL;
            }
            dispatch_semaphore_t ready = dispatch_semaphore_create(0);
            __block SCShareableContent *content;
            __block NSError *discoveryError;
            [contentClass getShareableContentExcludingDesktopWindows:NO onScreenWindowsOnly:YES
                                                  completionHandler:^(SCShareableContent *value, NSError *err) {
                content = value;
                discoveryError = err;
                dispatch_semaphore_signal(ready);
            }];
            if (dispatch_semaphore_wait(ready, dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC))) {
                SPError(error, errorSize, @"ScreenCaptureKit discovery timed out"); return NULL;
            }
            if (!content) { SPError(error, errorSize, discoveryError.localizedDescription); return NULL; }
            SCDisplay *display;
            for (SCDisplay *candidate in content.displays) {
                if (candidate.displayID == displayID) { display = candidate; break; }
            }
            if (!display) { SPError(error, errorSize, @"Selected display unavailable"); return NULL; }
            NSMutableArray<SCRunningApplication *> *excluded = [NSMutableArray array];
            for (SCRunningApplication *application in content.applications) {
                if (application.processID == getpid()) [excluded addObject:application];
            }
            if (!excluded.count) { SPError(error, errorSize, @"Cannot exclude SnapPin windows"); return NULL; }
            SCContentFilter *filter = [[filterClass alloc] initWithDisplay:display
                                                   excludingApplications:excluded exceptingWindows:@[]];
            SCStreamConfiguration *config = [[configClass alloc] init];
            config.sourceRect = CGRectMake(x, y, width, height);
            config.width = pixelWidth;
            config.height = pixelHeight;
            config.pixelFormat = kCVPixelFormatType_32BGRA;
            config.minimumFrameInterval = CMTimeMake(1, 30);
            config.queueDepth = 6;
            config.showsCursor = NO;
            SPLongStream *session = [SPLongStream new];
            session.outputQueue = dispatch_queue_create("com.snappin.long.frames", DISPATCH_QUEUE_SERIAL);
            session.callback = callback;
            session.context = context;
            session.stream = [[streamClass alloc] initWithFilter:filter configuration:config delegate:session];
            NSError *outputError;
            if (![session.stream addStreamOutput:session type:SCStreamOutputTypeScreen
                               sampleHandlerQueue:session.outputQueue error:&outputError]) {
                SPError(error, errorSize, outputError.localizedDescription); return NULL;
            }
            dispatch_semaphore_t started = dispatch_semaphore_create(0);
            __block NSError *startError;
            [session.stream startCaptureWithCompletionHandler:^(NSError *err) {
                startError = err;
                dispatch_semaphore_signal(started);
            }];
            BOOL timedOut = dispatch_semaphore_wait(started, dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC));
            if (timedOut || startError) {
                SPError(error, errorSize, timedOut ? @"ScreenCaptureKit start timed out" : startError.localizedDescription);
                SPStop(session);
                return NULL;
            }
            return (__bridge_retained void *)session;
        }
        SPError(error, errorSize, @"ScreenCaptureKit requires macOS 12.3");
        return NULL;
    }
}

void snappin_long_stream_stop(void *handle) {
    @autoreleasepool {
        if (@available(macOS 12.3, *)) {
            SPLongStream *session = (__bridge_transfer SPLongStream *)handle;
            if (session) SPStop(session);
        }
    }
}
