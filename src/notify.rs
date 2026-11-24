//! macOS notifications. Inside the installed app they go through the
//! system's notification centre, so they show as Wax with its icon and
//! obey System Settings → Notifications → Wax. A bare binary (cargo run)
//! has no bundle to post from, so it falls back to AppleScript.

use std::sync::Once;

/// True when running from inside an .app bundle.
pub fn in_bundle() -> bool {
    std::env::current_exe().map(|p| p.to_string_lossy().contains(".app/Contents/MacOS/")).unwrap_or(false)
}

/// Ask once for permission to show alerts, sounds and badges. macOS only
/// shows the prompt the first time; later calls return the saved answer.
pub fn request() {
    static ASKED: Once = Once::new();
    if !in_bundle() {
        return;
    }
    ASKED.call_once(|| {
        #[cfg(target_os = "macos")]
        mac::request();
    });
}

/// Post a notification now.
pub fn show(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    if in_bundle() {
        request();
        mac::show(title, body);
        return;
    }
    let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let script = format!("display notification {} with title {}", q(body), q(title));
    std::process::Command::new("osascript").args(["-e", &script]).output().ok();
}

#[cfg(target_os = "macos")]
mod mac {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNNotificationSound, UNUserNotificationCenter,
    };

    pub fn request() {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let done = RcBlock::new(|_granted: Bool, _err: *mut NSError| {});
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge,
            &done,
        );
    }

    pub fn show(title: &str, body: &str) {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let id = NSString::from_str(&format!("wax-{stamp}"));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
        center.addNotificationRequest_withCompletionHandler(&request, None);
    }
}
