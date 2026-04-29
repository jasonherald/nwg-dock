/// Edge detection threshold in pixels from the screen edge (autohide trigger zone).
pub const EDGE_THRESHOLD: i32 = 2;

/// Default dock background RGB (dark purple-gray). Embedded in the GTK4
/// compat CSS at startup and re-emitted whenever opacity changes (initial
/// load and hot-reload). User CSS files can override the full
/// `background-color` rule; this constant is just the built-in default.
pub const DEFAULT_BG_RGB: (u8, u8, u8) = (54, 54, 79);

/// Thickness of the Sway hotspot trigger window in pixels.
pub const HOTSPOT_THICKNESS: i32 = 4;

/// Pixel margin beyond the dock bounds before a drag-off triggers unpin.
pub const DRAG_OUTSIDE_MARGIN: f64 = 30.0;

/// Minimum pointer movement (in pixels) before a GestureDrag claims the
/// event sequence and suppresses Button::clicked. Matches GTK's default
/// DnD drag threshold. Without this, even 1px of movement during a click
/// would suppress the app launch (issue #30).
pub const DRAG_CLAIM_THRESHOLD: f64 = 8.0;

/// Maximum time (in seconds) to show the launch bounce animation.
/// After this, the animation stops even if no matching window appeared.
pub const LAUNCH_ANIMATION_TIMEOUT_SECS: u64 = 10;

/// Peak bounce height in pixels for the launch animation.
pub const LAUNCH_BOUNCE_HEIGHT_PX: i32 = 12;

/// Small downward dip in pixels at the bottom of the bounce cycle.
pub const LAUNCH_BOUNCE_DIP_PX: i32 = 4;

/// Duration of one full bounce cycle in milliseconds.
pub const LAUNCH_BOUNCE_DURATION_MS: i32 = 600;
