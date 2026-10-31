#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

const URL: &str = "https://www.facebook.com/messages/";

// Kills animations/transitions so the page stops constantly repainting.
const LITE_JS: &str = r#"
(function () {
  const css = `*,*::before,*::after{animation:none!important;transition:none!important;scroll-behavior:auto!important}
  video{display:none!important}`;
  const add = () => {
    if (document.getElementById('lite-css')) return;
    const s = document.createElement('style');
    s.id = 'lite-css'; s.textContent = css;
    (document.head || document.documentElement).appendChild(s);
  };
  add();
  document.addEventListener('DOMContentLoaded', add);
})();
"#;

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
            .initialization_script(LITE_JS)
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
