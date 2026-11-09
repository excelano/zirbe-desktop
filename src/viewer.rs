// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::borrow::Cow;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::WindowBuilder;
use wry::http::{Response, header};
use wry::{NewWindowResponse, WebViewBuilder};

pub const FLAG: &str = "--email-view";
const IMAGES: &str = "--images";

#[cfg(windows)]
const PAGE: &str = "http://zirbe.message/";
#[cfg(not(windows))]
const PAGE: &str = "zirbe://message/";

const BLOCKED: &str =
    "default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:";
const WITH_IMAGES: &str =
    "default-src 'none'; img-src data: https: http:; style-src 'unsafe-inline'; font-src data:";

pub fn open(title: &str, html: String, images: bool) -> Result<(), String> {
    let program = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut command = Command::new(program);
    command.arg(FLAG);
    if images {
        command.arg(IMAGES);
    }
    let mut child = command
        .arg(title)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| format!("the email window did not start: {error}"))?;
    let mut input = child.stdin.take().expect("a piped stdin");
    std::thread::spawn(move || {
        let _ = input.write_all(html.as_bytes());
        drop(input);
        let _ = child.wait();
    });
    Ok(())
}

pub fn run(arguments: &[String]) -> Result<(), String> {
    let images = arguments.iter().any(|argument| argument == IMAGES);
    let title = arguments
        .iter()
        .rfind(|argument| !argument.starts_with("--"))
        .map_or("Email", String::as_str);
    let mut html = String::new();
    std::io::stdin()
        .read_to_string(&mut html)
        .map_err(|error| error.to_string())?;
    let policy = if images { WITH_IMAGES } else { BLOCKED };
    let page = format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{policy}\">{html}");

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title(title)
        .with_inner_size(LogicalSize::new(860.0, 940.0))
        .build(&event_loop)
        .map_err(|error| error.to_string())?;
    let builder = WebViewBuilder::new()
        .with_custom_protocol("zirbe".into(), move |_, _| {
            Response::builder()
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CONTENT_SECURITY_POLICY, policy)
                .body(Cow::Owned(page.clone().into_bytes()))
                .expect("a response")
        })
        .with_url(PAGE)
        .with_devtools(false)
        .with_navigation_handler(|url| url.starts_with(PAGE) || outside(&url))
        .with_new_window_req_handler(|url, _| {
            outside(&url);
            NewWindowResponse::Deny
        })
        .with_download_started_handler(|_, _| false);

    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window);
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().expect("a window box"))
    };
    let webview = webview.map_err(|error| error.to_string())?;
    #[cfg(target_os = "linux")]
    {
        use webkit2gtk::{SettingsExt, WebViewExt};
        use wry::WebViewExtUnix;
        if let Some(settings) = webview.webview().settings() {
            settings.set_enable_javascript_markup(false);
        }
    }

    event_loop.run(move |event, _, flow| {
        let _ = &webview;
        *flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *flow = ControlFlow::Exit;
        }
    })
}

fn outside(url: &str) -> bool {
    if ["https:", "http:", "mailto:"]
        .iter()
        .any(|scheme| url.starts_with(scheme))
    {
        let _ = ::open::that_detached(url);
    }
    false
}
