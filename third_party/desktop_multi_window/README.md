This is desktop_multi_window 0.3.1, vendored from pub.dev under its LICENSE.

The Windows child-window runner disables Impeller, matching the main runner.
On the tested Windows machine, this reduced creation of two screenshot windows
from about 6 seconds to under 0.1 seconds. Keep this setting when updating the
package and recheck startup and first-use timings in the application log.
