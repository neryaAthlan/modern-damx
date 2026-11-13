pub mod boot;
pub mod credits;
pub mod keyboard;
pub mod performance;
pub mod status;
pub mod updates;

use adw::prelude::*;
use gtk::glib;
use std::rc::Rc;

pub struct Ctx {
    pub overlay: adw::ToastOverlay,
}

impl Ctx {
    pub fn toast(&self, message: impl AsRef<str>) {
        let toast = adw::Toast::new(message.as_ref());
        toast.set_timeout(4);
        self.overlay.add_toast(toast);
    }
}

/// Runs `work` on a plain thread and `done` back on the GTK main loop.
pub fn spawn<T, F, C>(work: F, done: C)
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    C: FnOnce(T) + 'static,
{
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(work());
    });
    glib::spawn_future_local(async move {
        if let Ok(value) = rx.recv().await {
            done(value);
        }
    });
}

/// Sends a privileged request and reports the outcome as a toast.
pub fn send(ctx: &Rc<Ctx>, request: damx_core::ipc::Request, success: &str) {
    let ctx = ctx.clone();
    let success = success.to_string();
    spawn(
        move || super::backend::request(request),
        move |result| match result {
            Ok(_) => ctx.toast(success),
            Err(err) => ctx.toast(format!("{err:#}")),
        },
    );
}

pub fn build(app: &adw::Application) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Modern DAMX")
        .default_width(900)
        .default_height(760)
        .build();

    let stack = adw::ViewStack::new();
    let switcher = adw::ViewSwitcher::builder()
        .policy(adw::ViewSwitcherPolicy::Wide)
        .stack(&stack)
        .build();

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));

    let overlay = adw::ToastOverlay::new();
    overlay.set_child(Some(&toolbar));
    window.set_content(Some(&overlay));

    let ctx = Rc::new(Ctx { overlay });

    add_page(&stack, &status::build(&ctx), "status", "Status", "computer-symbolic");
    add_page(
        &stack,
        &performance::build(&ctx),
        "performance",
        "Performance",
        "power-profile-performance-symbolic",
    );
    add_page(
        &stack,
        &keyboard::build(&ctx),
        "keyboard",
        "Keyboard",
        "keyboard-brightness-symbolic",
    );
    add_page(
        &stack,
        &boot::build(&ctx),
        "boot",
        "Boot preset",
        "system-reboot-symbolic",
    );
    add_page(
        &stack,
        &updates::build(&ctx),
        "updates",
        "Updates",
        "software-update-available-symbolic",
    );
    add_page(
        &stack,
        &credits::build(),
        "credits",
        "Credits",
        "help-about-symbolic",
    );

    window
}

fn add_page(
    stack: &adw::ViewStack,
    widget: &gtk::Widget,
    name: &str,
    title: &str,
    icon: &str,
) {
    let page = stack.add_titled(widget, Some(name), title);
    page.set_icon_name(Some(icon));
}
