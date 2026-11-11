#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

const URL: &str = "https://www.facebook.com/messages/";

const ADBLOCK_JS: &str = r##"
(function () {
  const isMsgs = () => /^\/messages/.test(location.pathname);
  const HEAD = 'h1,h2,h3,h4,[role="heading"]';
  const hide = (e) => e.style.setProperty('display', 'none', 'important');

  // --- Messenger-only: hide Facebook's top bar, only on the messages page ---
  const bar = document.createElement('style');
  bar.textContent = '[role="banner"]{display:none!important}';
  function syncBar() {
    if (isMsgs()) {
      if (!bar.isConnected) (document.head || document.documentElement).appendChild(bar);
    } else if (bar.isConnected) {
      bar.remove();
    }
  }
  syncBar();

  // --- block in-page jumps to the rest of Facebook (only from /messages) ---
  const OKPATH = /^\/(messages|messenger_media|login|checkpoint|recover|two_factor|cookie|privacy|dialog|ajax|api|security|help|device-based|confirmemail|plugins|rsrc|x\/oauth|legal|policies|settings|identity)/;
  const offLimits = (href) => {
    if (!isMsgs()) return false;
    try {
      const u = new URL(href, location.href);
      if (!/(^|\.)facebook\.com$/.test(u.hostname)) return false;
      return !OKPATH.test(u.pathname);
    } catch (e) { return false; }
  };
  document.addEventListener('click', (e) => {
    const a = e.target && e.target.closest && e.target.closest('a[href]');
    if (a && offLimits(a.getAttribute('href'))) {
      e.preventDefault();
      e.stopImmediatePropagation();
    }
  }, true);
  ['pushState', 'replaceState'].forEach((fn) => {
    const orig = history[fn];
    history[fn] = function (state, title, url) {
      if (url && offLimits(url)) return;
      const r = orig.apply(this, arguments);
      syncBar();
      return r;
    };
  });
  window.addEventListener('popstate', syncBar);

  // --- links to other sites (and window.open) go to the default browser ---
  const FAMILY = /(^|\.)(facebook\.com|messenger\.com|fbcdn\.net|fbsbx\.com|facebook\.net|fb\.com|fb\.me|meta\.com|instagram\.com)$/;
  const isExternal = (u) => {
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return false;
    if (!FAMILY.test(u.hostname)) return true;
    return /(^|\.)facebook\.com$/.test(u.hostname) && u.pathname.indexOf('/l.php') === 0;
  };
  // Navigating to these is intercepted by the app, which opens your browser.
  const toBrowser = (u) => { location.assign(u.href); };
  function extClick(e) {
    const a = e.target && e.target.closest && e.target.closest('a[href]');
    if (!a) return;
    let u;
    try { u = new URL(a.href, location.href); } catch (err) { return; }
    if (isExternal(u)) {
      e.preventDefault();
      e.stopImmediatePropagation();
      toBrowser(u);
    }
  }
  document.addEventListener('click', extClick, true);
  document.addEventListener('auxclick', extClick, true);
  const realOpen = window.open;
  window.open = function (url) {
    try {
      if (url) {
        const u = new URL(url, location.href);
        if (isExternal(u)) { toBrowser(u); return null; }
      }
    } catch (err) {}
    return realOpen.apply(this, arguments);
  };

  // --- hide "Sponsored" section headings (cheap: only looks at headings) ---
  const seen = new WeakSet();
  function hideSection(h) {
    let node = h;
    for (let i = 0; i < 10; i++) {
      const p = node.parentElement;
      if (!p || p === document.body || p === document.documentElement) break;
      if (p.querySelectorAll(HEAD).length > 1) break;
      node = p;
    }
    if (node === h) {
      let sib = node.nextElementSibling, n = 0;
      while (sib && n < 4 && !sib.matches(HEAD) && !sib.querySelector(HEAD)) {
        hide(sib); sib = sib.nextElementSibling; n++;
      }
    }
    hide(node);
  }
  function sweepHeadings() {
    if (document.hidden) return;
    document.querySelectorAll(HEAD).forEach((h) => {
      if (seen.has(h)) return;
      seen.add(h);
      if (h.textContent.trim().toLowerCase() === 'sponsored') hideSection(h);
    });
  }
  setInterval(sweepHeadings, 5000);
})();
"##;

static SUSPENDED_AT: AtomicU64 = AtomicU64::new(0);

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1)
        .max(1)
}

/// Hide + suspend the WebView2 page, or resume it. If it was suspended for
/// more than 2 minutes, reload on resume so Messenger reconnects cleanly
/// instead of sitting on a stuck "Sending..." state.
#[cfg(windows)]
fn set_suspended(win: &WebviewWindow, suspend: bool) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_3;
    use webview2_com::TrySuspendCompletedHandler;
    use windows::core::Interface;

    let mut reload = false;
    if suspend {
        if SUSPENDED_AT
            .compare_exchange(0, now_secs(), Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return; // already suspended
        }
    } else {
        let since = SUSPENDED_AT.swap(0, Ordering::SeqCst);
        if since == 0 {
            return; // wasn't suspended
        }
        reload = now_secs().saturating_sub(since) > 120;
    }

    let _ = win.with_webview(move |wv| unsafe {
        let controller = wv.controller();
        let Ok(core) = controller.CoreWebView2() else { return };
        let Ok(core3) = core.cast::<ICoreWebView2_3>() else { return };
        if suspend {
            let _ = controller.SetIsVisible(false);
            let handler = TrySuspendCompletedHandler::create(Box::new(|_, _| Ok(())));
            let _ = core3.TrySuspend(&handler);
        } else {
            let _ = core3.Resume();
            let _ = controller.SetIsVisible(true);
        }
    });
    if reload {
        let _ = win.eval("location.reload()");
    }
}

#[cfg(not(windows))]
fn set_suspended(_: &WebviewWindow, _: bool) {}

fn show(win: &WebviewWindow) {
    set_suspended(win, false);
    let _ = win.show();
    let _ = win.unminimize();
    let _ = win.set_focus();
}

/// Open a link in the user's normal browser.
fn open_external(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = url;
    }
}

/// Keep everything on Facebook's own sites inside the app (login, recovery and
/// security pages included). Only links to *other* sites open in your browser.
fn allow_nav(url: &Url) -> bool {
    if url.scheme() != "http" && url.scheme() != "https" {
        return true;
    }
    let host = url.host_str().unwrap_or("");
    let path = url.path();
    let is = |d: &str| host == d || host.ends_with(&format!(".{d}"));

    if is("facebook.com") && path.starts_with("/l.php") {
        if let Some((_, u)) = url.query_pairs().find(|(k, _)| &**k == "u") {
            open_external(&u);
        }
        return false;
    }
    const FAMILY: &[&str] = &[
        "facebook.com", "messenger.com", "fbcdn.net", "fbsbx.com",
        "facebook.net", "fb.com", "fb.me", "meta.com", "instagram.com",
    ];
    if FAMILY.iter().any(|d| is(*d)) {
        return true;
    }
    open_external(url.as_str());
    false
}

fn main() {
    tauri::Builder::default()
        // Must be first: a second launch just brings the existing window back.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                show(&w);
            }
        }))
        .setup(|app| {
            let win = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(URL.parse().unwrap()),
            )
            .title("Messenger")
            .inner_size(1100.0, 750.0)
            .on_navigation(|url| allow_nav(url))
            .disable_drag_drop_handler()
            .initialization_script(ADBLOCK_JS)
            .build()?;

            let w = win.clone();
            win.on_window_event(move |e| match e {
                WindowEvent::Resized(_) => {
                    set_suspended(&w, w.is_minimized().unwrap_or(false));
                }
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = w.hide();
                    set_suspended(&w, true);
                }
                _ => {}
            });

            let open = MenuItem::with_id(app, "open", "Open Messenger", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Lite Messenger")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, ev| match ev.id.as_ref() {
                    "open" => {
                        if let Some(w) = app.get_webview_window("main") {
                            show(&w);
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, ev| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = ev
                    {
                        if let Some(w) = tray.app_handle().get_webview_window("main") {
                            show(&w);
                        }
                    }
                })
                .build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running app");
}
