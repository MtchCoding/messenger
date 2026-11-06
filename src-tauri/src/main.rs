#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

const URL: &str = "https://www.facebook.com/messages/";



const ADBLOCK_JS: &str = r##"
(function () {
  // 1) Block known tracker/ad requests made from page JavaScript
  const BLOCKED = [
    'doubleclick.net', 'googlesyndication.com', 'google-analytics.com',
    'googletagmanager.com', 'pixel.facebook.com', 'an.facebook.com',
    'facebook.com/tr'
  ];
  const isBlocked = (u) => {
    try { u = String(u); } catch (e) { return false; }
    return BLOCKED.some((b) => u.indexOf(b) !== -1);
  };
  const realFetch = window.fetch;
  if (realFetch) {
    window.fetch = function (input) {
      const url = input && input.url ? input.url : input;
      if (isBlocked(url)) return Promise.resolve(new Response('', { status: 204 }));
      return realFetch.apply(this, arguments);
    };
  }
  const realBeacon = navigator.sendBeacon && navigator.sendBeacon.bind(navigator);
  if (realBeacon) {
    navigator.sendBeacon = function (url) {
      return isBlocked(url) ? true : realBeacon.apply(null, arguments);
    };
  }

  // 2) Hide anything labelled "Sponsored" - only checks newly added nodes
  const queue = new Set();
  let pending = false;
  const HEAD = 'h1,h2,h3,h4,[role="heading"]';
  const SEL = 'span,a,h1,h2,h3,h4,[role="heading"]';
  const hide = (e) => e.style.setProperty('display', 'none', 'important');
  function hideSection(h) {
    // climb while the wrapper still contains only this one heading
    let node = h;
    for (let i = 0; i < 10; i++) {
      const p = node.parentElement;
      if (!p || p === document.body || p === document.documentElement) break;
      if (p.querySelectorAll(HEAD).length > 1) break;
      node = p;
    }
    if (node === h) {
      // heading and ads are siblings: hide the siblings that follow it
      let sib = node.nextElementSibling, n = 0;
      while (sib && n < 4 && !sib.matches(HEAD) && !sib.querySelector(HEAD)) {
        hide(sib); sib = sib.nextElementSibling; n++;
      }
    }
    hide(node);
  }
  function check(el) {
    if (el.children.length !== 0) return;
    if (el.textContent.trim().toLowerCase() !== 'sponsored') return;
    const h = el.closest(HEAD);
    if (h) { hideSection(h); return; }
    const row = el.closest('[role="row"],[role="listitem"],[role="article"],li');
    if (row) hide(row);
  }
  function sweep() {
    pending = false;
    const nodes = Array.from(queue);
    queue.clear();
    nodes.forEach((n) => {
      if (!n.isConnected) return;
      if (n.matches && n.matches(SEL)) check(n);
      if (n.querySelectorAll) n.querySelectorAll(SEL).forEach(check);
    });
  }
  function start() {
    new MutationObserver((muts) => {
      for (const m of muts)
        for (const n of m.addedNodes) if (n.nodeType === 1) queue.add(n);
      if (!pending && queue.size) { pending = true; setTimeout(sweep, 2000); }
    }).observe(document.documentElement, { childList: true, subtree: true });
    queue.add(document.documentElement);
    pending = true;
    setTimeout(sweep, 2000);
  }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start);
  } else {
    start();
  }
})();

"##;

/// Hide + suspend the WebView2 page (stops JS timers, rendering, network
/// activity), or resume it. This is where the real CPU savings come from.
#[cfg(windows)]
fn set_suspended(win: &WebviewWindow, suspend: bool) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_3;
    use webview2_com::TrySuspendCompletedHandler;
    use windows::core::Interface;

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
}

#[cfg(not(windows))]
fn set_suspended(_: &WebviewWindow, _: bool) {}

fn show(win: &WebviewWindow) {
    set_suspended(win, false);
    let _ = win.show();
    let _ = win.unminimize();
    let _ = win.set_focus();
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let win = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(URL.parse().unwrap()),
            )
            .title("Messenger")
            .inner_size(1100.0, 750.0)
            .initialization_script(ADBLOCK_JS)
            // Replaces Tauri's defaults, so keep those and add our own.
            .additional_browser_args(
                "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,Translate \
                 --disable-background-networking --disable-component-update",
            )
            .build()?;

            // Suspend when minimized, resume when restored.
            let w = win.clone();
            win.on_window_event(move |e| match e {
                WindowEvent::Resized(_) => {
                    set_suspended(&w, w.is_minimized().unwrap_or(false));
                }
                WindowEvent::CloseRequested { api, .. } => {
                    // Close button -> hide to tray + suspend.
                    api.prevent_close();
                    let _ = w.hide();
                    set_suspended(&w, true);
                }
                _ => {}
            });

            // Tray icon
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
                        if let Some(w) = app.get_webview_window("main") { show(&w); }
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
