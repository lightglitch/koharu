use std::sync::Arc;

use sentry::{ClientOptions, User, integrations::minidump::MinidumpIntegration};

/// Starts Sentry with everything on: every error and trace, logs, native
/// crash dumps, release health, and the interface's events through
/// `koharu-sentry`. It must run before the async runtime starts, so its
/// threads share the main hub, and before anything that should run only once:
/// the crash reporter re-runs this binary and never returns from here.
pub fn initialize() -> sentry::ClientInitGuard {
    // The crash reporter has its own scope: a native crash carries only the
    // breadcrumbs and user sent to it, through the integration the client
    // shares with this function.
    let minidump = Arc::new(MinidumpIntegration::new());
    let forward = Arc::clone(&minidump);
    let mut options = ClientOptions::new()
        .maybe_release(sentry::release_name!())
        .send_default_pii(true)
        .sample_rate(1.0)
        .traces_sample_rate(1.0)
        .auto_session_tracking(true)
        .before_breadcrumb(move |breadcrumb| {
            forward.add_breadcrumb(breadcrumb.clone());
            Some(breadcrumb)
        })
        .transport(koharu_sentry::Transport)
        .add_integration(koharu_sentry::Sentry::default());
    // Fork divergence: Sentry is off on this fork. The client starts, as the
    // Sentry plugin needs one, but without a DSN it sends nothing whatever
    // `SENTRY_DSN` a build has, and without the minidump integration no crash
    // reporter process starts.
    options.dsn = None;
    let guard = sentry::init(options);

    let user = User {
        id: machine_uid::get().ok(),
        ..Default::default()
    };
    sentry::configure_scope(|scope| scope.set_user(Some(user.clone())));
    minidump.set_user(Some(user));
    guard
}
