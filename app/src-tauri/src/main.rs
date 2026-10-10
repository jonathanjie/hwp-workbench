//! Native window glue. All document operations live in app-core.
use app_core::queue::CommandQueue;
use app_core::{Backend, Dialogs, Error, Events, Shell};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{Emitter, Manager, Runtime};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogResult};
#[cfg(test)]
mod command_names;
mod launch;
#[cfg(test)]
mod tests;

fn native_error(error: impl std::fmt::Display) -> Error {
    Error::new(error.to_string())
}
struct NativeHost<R: Runtime> {
    app: tauri::AppHandle<R>,
}
impl<R: Runtime> Dialogs for NativeHost<R> {
    fn open_file(&self) -> app_core::Result<Option<PathBuf>> {
        self.app
            .dialog()
            .file()
            .add_filter("HWP / HWPX", &["hwp", "hwpx"])
            .blocking_pick_file()
            .map(|file| file.into_path().map_err(native_error))
            .transpose()
    }
    fn save_file(
        &self,
        default: &Path,
        extensions: &[String],
    ) -> app_core::Result<Option<PathBuf>> {
        let parent = default
            .parent()
            .ok_or_else(|| Error::new("Unsupported document"))?;
        let name = default
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Error::new("Unsupported document"))?;
        let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
        self.app
            .dialog()
            .file()
            .set_directory(parent)
            .set_file_name(name)
            .add_filter("HWP / HWPX / JSON", &extensions)
            .blocking_save_file()
            .map(|file| file.into_path().map_err(native_error))
            .transpose()
    }
    fn message(&self, message: &str, buttons: &[&str], default: usize) -> app_core::Result<usize> {
        let order = dialog_order(buttons, default)?;
        let labels: Vec<_> = order
            .iter()
            .filter_map(|index| buttons.get(*index).copied())
            .collect();
        let native_buttons = match labels.as_slice() {
            [first, second] => {
                MessageDialogButtons::OkCancelCustom((*first).into(), (*second).into())
            }
            [first, second, third] => MessageDialogButtons::YesNoCancelCustom(
                (*first).into(),
                (*second).into(),
                (*third).into(),
            ),
            _ => return Err(Error::new("Invalid message buttons")),
        };
        let answer = self
            .app
            .dialog()
            .message(message)
            .title("HWP Workbench")
            .buttons(native_buttons)
            .blocking_show_with_result();
        Ok(dialog_result(buttons, &order, answer, default))
    }
}
// rfd exposes no default-button setter. Put the safe default first, then map
// the selected label back to the original Electron button index.
fn dialog_order(buttons: &[&str], default: usize) -> app_core::Result<Vec<usize>> {
    if buttons.get(default).is_none() {
        return Err(Error::new("Invalid message buttons"));
    }
    let mut order = vec![default];
    order.extend((0..buttons.len()).filter(|index| *index != default));
    Ok(order)
}
fn dialog_result(
    buttons: &[&str],
    order: &[usize],
    answer: MessageDialogResult,
    default: usize,
) -> usize {
    match answer {
        MessageDialogResult::Custom(label) => buttons
            .iter()
            .position(|button| *button == label)
            .unwrap_or(default),
        MessageDialogResult::Ok | MessageDialogResult::Yes => {
            order.first().copied().unwrap_or(default)
        }
        MessageDialogResult::No => order.get(1).copied().unwrap_or(default),
        MessageDialogResult::Cancel => buttons
            .iter()
            .position(|label| label.starts_with("Cancel / "))
            .unwrap_or(default),
    }
}
impl<R: Runtime> Events for NativeHost<R> {
    fn emit(&self, event: &str, payload: Value) -> app_core::Result<()> {
        self.app
            .emit_to("main", event, payload)
            .map_err(native_error)
    }
    fn focus(&self) -> app_core::Result<()> {
        if let Some(window) = self.app.get_webview_window("main") {
            window.unminimize().map_err(native_error)?;
            window.show().map_err(native_error)?;
            window.set_focus().map_err(native_error)?;
        }
        Ok(())
    }
    fn key_scheme(&self, scheme: &str) -> app_core::Result<()> {
        self.app
            .set_menu(menu(&self.app, scheme).map_err(native_error)?)
            .map_err(native_error)?;
        Ok(())
    }
}
impl<R: Runtime> Shell for NativeHost<R> {
    fn copy(&self, text: &str) -> app_core::Result<()> {
        self.app.clipboard().write_text(text).map_err(native_error)
    }
    fn reveal(&self, file: &Path) -> app_core::Result<()> {
        // Fixed Finder action, never a shell, and never a renderer-supplied path.
        #[cfg(target_os = "macos")]
        {
            let status = std::process::Command::new("/usr/bin/open")
                .arg("-R")
                .arg(file)
                .status()?;
            if !status.success() {
                return Err(Error::new("Finder reveal failed"));
            }
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = file;
            Err(Error::new("Finder reveal is available on macOS"))
        }
    }
}
fn allowed_navigation(url: &tauri::Url) -> bool {
    let bundled_origin = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost"));
    bundled_origin
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        // Tauri serves index.html at tauri://localhost, whose URL path is empty.
        && ["", "/", "/index.html"].contains(&url.path())
        && url.query().is_none()
}
enum Work {
    Queued(app_core::queue::Response),
    Preview(Box<app_core::PreviewRequest>),
}
// Enter on IPC receipt. Async command macros would postpone FIFO reservation
// until a runtime task is polled, allowing a native close event to overtake edits.
fn dispatch<R: Runtime>(invoke: tauri::ipc::Invoke<R>, command: &'static str) {
    let webview = invoke.message.webview();
    let prepared = (|| -> app_core::Result<_> {
        if webview.label() != "main" || !allowed_navigation(&webview.url().map_err(native_error)?) {
            return Err(Error::new("Unauthorized IPC sender"));
        }
        let args = match invoke.message.payload() {
            tauri::ipc::InvokeBody::Json(args) => args.clone(),
            _ => return Err(Error::new("Invalid desktop arguments")),
        };
        let backend =
            backend(webview.app_handle()).ok_or_else(|| Error::new("Desktop state unavailable"))?;
        let app = webview.app_handle();
        let queue = app
            .try_state::<CommandQueue>()
            .ok_or_else(|| Error::new("Desktop queue unavailable"))?;
        if !queue.is_open() {
            return Err(Error::new("Desktop queue closed"));
        }
        let work = if command == "document_preview" {
            Work::Preview(Box::new(backend.prepare_preview(&args)?))
        } else {
            Work::Queued(queue.invoke(command, args)?)
        };
        Ok((backend, work))
    })();
    let (backend, work) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            invoke.resolver.reject(error.message);
            return;
        }
    };
    invoke.resolver.respond_async(async move {
        tauri::async_runtime::spawn_blocking(move || match work {
            Work::Queued(response) => response.recv().map_err(native_error)?,
            Work::Preview(request) => backend.run_preview(*request),
        })
        .await
        .map_err(tauri::ipc::InvokeError::from_error)?
        .map_err(tauri::ipc::InvokeError::from_error)
    });
}
macro_rules! desktop_commands {
    ($($name:ident),* $(,)?) => {
        $(fn $name<R: Runtime>(invoke: tauri::ipc::Invoke<R>) {
            dispatch(invoke,stringify!($name));
        })*
        fn commands<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
            builder.invoke_handler(|invoke| match invoke.message.command() {
                $(stringify!($name) => { $name(invoke); true },)*
                _ => false,
            })
        }
    }
}
desktop_commands![
    document_open,
    document_open_dropped,
    document_demo,
    document_bytes,
    document_recent,
    document_open_recent,
    document_cache_thumbnail,
    document_preview,
    document_format,
    document_plan,
    document_save,
    document_save_in_place,
    document_export_patch,
    document_draft,
    shell_reveal_previous,
    shell_reveal_output,
    agents_status,
    agents_install_skill,
    agents_copy_mcp,
    agents_reveal_bundle,
    document_handoff_ready,
    document_editing,
    agent_link,
    agent_selection,
    agent_decide,
    keyboard_scheme,
    clipboard_copy
];

fn accelerator(id: &str, scheme: &str) -> tauri::Result<Option<String>> {
    let keymap: Value =
        serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/keymap.json")))?;
    Ok(keymap
        .get(scheme)
        .and_then(|map| map.get(id))
        .and_then(Value::as_str)
        .map(str::to_owned))
}
fn menu<R: Runtime>(
    app: &tauri::AppHandle<R>,
    scheme: &str,
) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem as Native, Submenu};
    let item =
        |id: &str, label: &str| MenuItem::with_id(app, id, label, true, accelerator(id, scheme)?);
    let application = Submenu::with_items(
        app,
        "HWP Workbench",
        true,
        &[
            &Native::about(app, None, None)?,
            &Native::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quit / 종료", true, Some("Command+Q"))?,
        ],
    )?;
    let file = Submenu::with_items(
        app,
        "File / 파일",
        true,
        &[
            &item("open", "Open / 불러오기")?,
            &item("save", "Save / 저장")?,
            &item("saveCopy", "Save As / 다른 이름으로 저장")?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit / 편집",
        true,
        &[
            &item("undo", "Undo / 되돌리기")?,
            &item("redo", "Redo / 다시 실행")?,
            &Native::separator(app)?,
            &Native::cut(app, None)?,
            &Native::copy(app, None)?,
            &Native::paste(app, None)?,
            &Native::select_all(app, None)?,
            &item("find", "Find / 찾기")?,
            &item("replace", "Find and replace / 찾아 바꾸기")?,
        ],
    )?;
    let view = Submenu::with_items(
        app,
        "View / 보기",
        true,
        &[
            &item("palette", "Commands / 명령")?,
            &item("shortcuts", "Keyboard shortcuts / 단축키")?,
            &MenuItem::with_id(app, "language", "Language / 언어", true, None::<&str>)?,
            &Native::fullscreen(app, None)?,
        ],
    )?;
    Menu::with_items(app, &[&application, &file, &edit, &view])
}
fn backend<R: Runtime>(app: &tauri::AppHandle<R>) -> Option<Arc<Backend>> {
    app.try_state::<Arc<Backend>>()
        .map(|state| state.inner().clone())
}
// The single-instance listener can receive a request before setup finishes.
#[derive(Default)]
struct PendingOpens(Mutex<Vec<PathBuf>>);
fn submit_open<R: Runtime>(app: &tauri::AppHandle<R>, file: PathBuf) {
    if let Some(pending) = app.try_state::<Arc<PendingOpens>>() {
        if let Ok(mut pending) = pending.inner().0.lock() {
            if let Some(queue) = app.try_state::<CommandQueue>() {
                if let Err(error) = queue.open(file) {
                    eprintln!("Open handoff: {}", error.message);
                }
            } else {
                pending.push(file);
            }
        }
    }
}
#[derive(Default)]
struct Shutdown {
    requesting: AtomicBool,
    approved: AtomicBool,
}
fn request_close<R: Runtime>(app: &tauri::AppHandle<R>) {
    let Some(shutdown) = app.try_state::<Arc<Shutdown>>() else {
        return;
    };
    if shutdown.requesting.swap(true, Ordering::SeqCst) {
        return;
    }
    let shutdown = shutdown.inner().clone();
    let response = app
        .try_state::<CommandQueue>()
        .ok_or_else(|| Error::new("Desktop queue unavailable"))
        .and_then(|queue| queue.close());
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let approved = response
            .and_then(|receiver| receiver.recv().map_err(native_error)?)
            .map(|value| value == Value::Bool(true));
        if matches!(approved, Ok(true)) {
            shutdown.approved.store(true, Ordering::SeqCst);
            if let Some(backend) = backend(&app) {
                backend.stop();
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.destroy();
            }
            app.exit(0);
        } else {
            shutdown.requesting.store(false, Ordering::SeqCst);
        }
    });
}
/// Folder that holds runtime resources: the bundle's `app` resource folder,
/// or the source checkout. An unbundled development binary has no resource
/// folder (`resource_dir()` fails), so `resource_dir` is `None` there.
fn engine_source_root(resource_dir: Option<PathBuf>) -> std::io::Result<PathBuf> {
    let bundled = resource_dir
        .map(|directory| directory.join("app"))
        .filter(|directory| directory.join("skills/hwp-workbench/SKILL.md").is_file());
    match bundled {
        Some(directory) => Ok(directory),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize(),
    }
}
fn window_color(theme: tauri::Theme) -> tauri::window::Color {
    if theme == tauri::Theme::Dark {
        tauri::window::Color(15, 15, 14, 255)
    } else {
        tauri::window::Color(232, 231, 227, 255)
    }
}
fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let builder = commands(tauri::Builder::default())
        .manage(Arc::new(PendingOpens::default()))
        .manage(Arc::new(Shutdown::default()))
        .plugin(tauri_plugin_single_instance::init(
            |app, args, _directory| {
                if let Some(file) = app_core::open_argument(&args) {
                    submit_open(app, file);
                }
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            },
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .on_menu_event(|app, event| {
            let command = event.id().as_ref();
            if command == "quit" {
                request_close(app);
                return;
            }
            if [
                "open",
                "save",
                "saveCopy",
                "undo",
                "redo",
                "find",
                "replace",
                "palette",
                "shortcuts",
                "language",
            ]
            .contains(&command)
            {
                let _ = app.emit_to("main", "app:command", command);
            }
        })
        .on_webview_event(|webview, event| {
            if webview.label() != "main" {
                return;
            }
            if let tauri::WebviewEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                if let Some(backend) = backend(webview.app_handle()) {
                    if let Err(error) = backend.grant_drop(paths.clone()) {
                        eprintln!("File drop: {}", error.message);
                    }
                }
            }
        })
        .on_page_load(|webview, event| {
            if webview.label() == "main" && event.event() == tauri::webview::PageLoadEvent::Started
            {
                if let Some(backend) = backend(webview.app_handle()) {
                    let _ = backend.renderer_loading();
                }
            }
        })
        .setup(|app| {
            let source_root = engine_source_root(app.path().resource_dir().ok())?;
            let profile = app.path().app_data_dir()?;
            let host = Arc::new(NativeHost {
                app: app.handle().clone(),
            });
            let engine = app_core::engine::desktop_engine(source_root.clone());
            let backend = Arc::new(Backend::new(
                engine,
                host.clone(),
                host.clone(),
                host,
                source_root,
                profile,
            )?);
            let scheme = backend.key_scheme();
            // Hold pending opens while installing the queue, so startup handoffs
            // cannot overtake files already received by the native listener.
            {
                let pending = app.state::<Arc<PendingOpens>>();
                let mut pending = app_core::lock(&pending.inner().0)?;
                let queue = CommandQueue::new(backend.clone());
                for file in pending.drain(..) {
                    let _ = queue.open(file)?;
                }
                app.manage(queue);
                app.manage(backend.clone());
            }
            app.set_menu(menu(app.handle(), &scheme)?)?;
            let mut window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("HWP Workbench")
            .inner_size(1400.0, 930.0)
            .min_inner_size(1100.0, 720.0)
            .background_color(window_color(tauri::Theme::Light))
            .devtools(cfg!(debug_assertions))
            .on_navigation(allowed_navigation)
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny);
            #[cfg(target_os = "macos")]
            {
                window = window
                    .title_bar_style(tauri::TitleBarStyle::Overlay)
                    .hidden_title(true)
                    .traffic_light_position(tauri::LogicalPosition::new(18.0, 18.0));
            }
            let window = window.build()?;
            window.set_background_color(Some(window_color(window.theme()?)))?;
            let close_app = app.handle().clone();
            window.on_window_event(move |event| match event {
                tauri::WindowEvent::ThemeChanged(theme) => {
                    if let Some(window) = close_app.get_webview_window("main") {
                        let _ = window.set_background_color(Some(window_color(*theme)));
                    }
                }
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    request_close(&close_app);
                }
                _ => (),
            });
            if let Some(file) = app_core::open_argument(&std::env::args().collect::<Vec<_>>()) {
                submit_open(app.handle(), file);
            }
            Ok(())
        });
    builder
        .build(tauri::generate_context!())?
        .run(|app, event| match event {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            tauri::RunEvent::Opened { urls } => {
                for url in urls {
                    if let Ok(file) = url.to_file_path() {
                        submit_open(app, file);
                    }
                }
            }
            tauri::RunEvent::ExitRequested { api, .. } => {
                let approved = app
                    .try_state::<Arc<Shutdown>>()
                    .is_some_and(|shutdown| shutdown.approved.load(Ordering::SeqCst));
                if !approved {
                    api.prevent_exit();
                    request_close(app);
                }
            }
            tauri::RunEvent::Exit => {
                if let Some(backend) = backend(app) {
                    backend.stop();
                }
            }
            _ => (),
        });
    Ok(())
}
fn main() {
    let mut arguments = std::env::args_os();
    let argv0 = arguments.next().unwrap_or_default();
    let args: Vec<_> = arguments.collect();
    match launch::launch_mode(Path::new(&argv0), &args) {
        launch::LaunchMode::Worker | launch::LaunchMode::Cli => {
            // Shared CLI dispatch also handles the worker before any Tauri setup.
            std::process::exit(hwp_workbench::cli::main());
        }
        launch::LaunchMode::Desktop => {}
    }
    if let Err(error) = run() {
        eprintln!("HWP Workbench: {error}");
        std::process::exit(1);
    }
}
