// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::HashSet;
use std::io::Read;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use blitz_dom::{Attribute, BaseDocument, DocumentConfig, QualName, local_name, ns};
use blitz_html::HtmlDocument;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use eframe::egui::{
    self, Color32, ColorImage, CursorIcon, Rect, Sense, TextureHandle, TextureOptions, Ui, pos2,
    vec2,
};

const TILE: u32 = 2048;
const TALLEST: u32 = 40_000;
const IMAGE_LIMIT: u64 = 10 * 1024 * 1024;
const IMAGE_TIMEOUT: Duration = Duration::from_secs(20);
const BLOCKED_STYLE: &str = "display: inline-block; border: 1px solid #c8c8c8; padding: 2px 6px; \
     color: #666; font: 12px sans-serif";

enum Command {
    Render { width: u32, scale: f32 },
    Point { x: f32, y: f32, click: bool },
    Loaded,
}

struct Rendered {
    width: u32,
    scale: f32,
    height: u32,
    tiles: Vec<ColorImage>,
}

enum Reply {
    Rendered(Rendered),
    Link { href: Option<String>, click: bool },
}

pub struct Page {
    commands: Sender<Command>,
    replies: Receiver<Reply>,
    tiles: Vec<(TextureHandle, f32)>,
    shown: Option<(u32, f32)>,
    asked: Option<(u32, f32)>,
    height: f32,
    hovered: Option<String>,
    pointer: Option<egui::Pos2>,
}

impl Page {
    pub fn new(html: String, images: bool) -> Page {
        let (commands, inbox) = mpsc::channel();
        let (outbox, replies) = mpsc::channel();
        let wake = commands.clone();
        std::thread::Builder::new()
            .name("email view".into())
            .spawn(move || Renderer::new(html, images, wake, outbox).run(inbox))
            .expect("a thread for the email view");
        Page {
            commands,
            replies,
            tiles: Vec::new(),
            shown: None,
            asked: None,
            height: 0.0,
            hovered: None,
            pointer: None,
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        loop {
            match self.replies.try_recv() {
                Ok(Reply::Rendered(rendered)) => {
                    self.height = rendered.height as f32 / rendered.scale;
                    let mut top = 0.0;
                    self.tiles = rendered
                        .tiles
                        .into_iter()
                        .enumerate()
                        .map(|(index, image)| {
                            let height = image.size[1] as f32 / rendered.scale;
                            let texture = ctx.load_texture(
                                format!("email tile {index}"),
                                image,
                                TextureOptions::LINEAR,
                            );
                            let placed = (texture, top);
                            top += height;
                            placed
                        })
                        .collect();
                    self.shown = Some((rendered.width, rendered.scale));
                }
                Ok(Reply::Link { href, click }) => {
                    if click {
                        if let Some(href) = &href {
                            let _ = open::that_detached(href);
                        }
                    } else {
                        self.hovered = href;
                    }
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }

        let scale = ctx.pixels_per_point();
        let width = ui.available_width().max(200.0);
        let wanted = ((width * scale).round() as u32, scale);
        if self.asked != Some(wanted) {
            self.asked = Some(wanted);
            let _ = self.commands.send(Command::Render {
                width: wanted.0,
                scale,
            });
        }
        if self.shown != self.asked {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if self.tiles.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| ui.spinner());
            return;
        }

        let shown_width = self
            .shown
            .map_or(width, |(pixels, scale)| pixels as f32 / scale);
        let (rect, response) =
            ui.allocate_exact_size(vec2(shown_width, self.height), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::WHITE);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        for (texture, top) in &self.tiles {
            let size = texture.size_vec2() / self.shown.map_or(1.0, |(_, scale)| scale);
            let spot = Rect::from_min_size(rect.min + vec2(0.0, *top), size);
            if ui.is_rect_visible(spot) {
                painter.image(texture.id(), spot, uv, Color32::WHITE);
            }
        }

        if let Some(at) = response.hover_pos() {
            let inside = at - rect.min;
            if self.pointer != Some(at) {
                self.pointer = Some(at);
                let _ = self.commands.send(Command::Point {
                    x: inside.x,
                    y: inside.y,
                    click: false,
                });
            }
            if let Some(href) = &self.hovered {
                ctx.set_cursor_icon(CursorIcon::PointingHand);
                response.clone().on_hover_text_at_pointer(href);
            }
            if response.clicked() {
                let _ = self.commands.send(Command::Point {
                    x: inside.x,
                    y: inside.y,
                    click: true,
                });
            }
        } else {
            self.pointer = None;
            self.hovered = None;
        }
    }
}

struct Renderer {
    html: String,
    images: bool,
    wake: Sender<Command>,
    outbox: Sender<Reply>,
    document: Option<HtmlDocument>,
    size: Option<(u32, f32)>,
}

impl Renderer {
    fn new(html: String, images: bool, wake: Sender<Command>, outbox: Sender<Reply>) -> Renderer {
        Renderer {
            html,
            images,
            wake,
            outbox,
            document: None,
            size: None,
        }
    }

    fn run(mut self, inbox: Receiver<Command>) {
        while let Ok(command) = inbox.recv() {
            let mut render = None;
            let mut loaded = false;
            let mut queue = vec![command];
            queue.extend(inbox.try_iter());
            for command in queue {
                match command {
                    Command::Render { width, scale } => render = Some((width, scale)),
                    Command::Loaded => loaded = true,
                    Command::Point { x, y, click } => {
                        let href = self.link_at(x, y);
                        if self.outbox.send(Reply::Link { href, click }).is_err() {
                            return;
                        }
                    }
                }
            }
            if let Some(size) = render {
                self.size = Some(size);
            }
            if (render.is_some() || loaded)
                && let Some((width, scale)) = self.size
            {
                let rendered = self.render(width, scale);
                if self.outbox.send(Reply::Rendered(rendered)).is_err() {
                    return;
                }
            }
        }
    }

    fn document(&mut self, width: u32, scale: f32) -> &mut HtmlDocument {
        if self.document.is_none() {
            let config = |allowed: HashSet<String>| DocumentConfig {
                viewport: Some(Viewport::new(width, 800, scale, ColorScheme::Light)),
                net_provider: Some(Arc::new(Provider {
                    allowed,
                    wake: self.wake.clone(),
                })),
                ..DocumentConfig::default()
            };
            let offline = HtmlDocument::from_html(&self.html, config(HashSet::new()));
            let mut document = if self.images {
                HtmlDocument::from_html(&self.html, config(image_sources(&offline)))
            } else {
                offline
            };
            if !self.images {
                label_blocked_images(&mut document);
            }
            self.document = Some(document);
        }
        self.document.as_mut().expect("a document")
    }

    fn render(&mut self, width: u32, scale: f32) -> Rendered {
        let document = self.document(width, scale);
        document.handle_messages();
        document.set_viewport(Viewport::new(width, 800, scale, ColorScheme::Light));
        document.resolve(0.0);
        let css_height = document.root_element().final_layout().size.height;
        let height = ((css_height * scale).ceil() as u32).clamp(1, TALLEST);
        document.set_viewport(Viewport::new(width, height, scale, ColorScheme::Light));
        document.resolve(0.0);
        let tiles = (0..height)
            .step_by(TILE as usize)
            .map(|top| {
                let tall = TILE.min(height - top);
                let pixels = render_to_buffer::<VelloCpuImageRenderer, _>(
                    |scene| {
                        blitz_paint::paint_scene(scene, document, scale as f64, width, tall, 0, top)
                    },
                    width,
                    tall,
                );
                ColorImage::from_rgba_premultiplied([width as usize, tall as usize], &pixels)
            })
            .collect();
        Rendered {
            width,
            scale,
            height,
            tiles,
        }
    }

    fn link_at(&self, x: f32, y: f32) -> Option<String> {
        let document = self.document.as_ref()?;
        let mut id = document.hit(x, y)?.node_id;
        loop {
            let node = document.get_node(id)?;
            if node.data.is_element_with_tag_name(&local_name!("a"))
                && let Some(href) = node.data.attr(local_name!("href"))
            {
                let href = href.trim();
                return ["https:", "http:", "mailto:"]
                    .iter()
                    .any(|scheme| {
                        href.len() > scheme.len()
                            && href[..scheme.len()].eq_ignore_ascii_case(scheme)
                    })
                    .then(|| href.to_string());
            }
            id = node.parent?;
        }
    }
}

fn remote(source: &str) -> bool {
    let source = source.trim_start();
    !source.is_empty()
        && !source
            .get(..5)
            .is_some_and(|start| start.eq_ignore_ascii_case("data:"))
}

fn image_sources(document: &BaseDocument) -> HashSet<String> {
    document
        .tree()
        .iter()
        .filter(|(_, node)| node.data.is_element_with_tag_name(&local_name!("img")))
        .filter_map(|(_, node)| node.data.attr(local_name!("src")))
        .filter_map(|source| url::Url::parse(source.trim()).ok())
        .filter(|url| matches!(url.scheme(), "https" | "http"))
        .map(String::from)
        .collect()
}

fn label_blocked_images(document: &mut BaseDocument) {
    let blocked: Vec<(blitz_dom::NodeId, String)> = document
        .tree()
        .iter()
        .filter(|(_, node)| node.data.is_element_with_tag_name(&local_name!("img")))
        .filter(|(_, node)| node.data.attr(local_name!("src")).is_some_and(remote))
        .map(|(id, node)| {
            let alt = node
                .data
                .attr(local_name!("alt"))
                .unwrap_or_default()
                .trim();
            (id, alt.to_string())
        })
        .collect();
    let mut mutator = document.mutate();
    for (id, alt) in blocked {
        if alt.is_empty() {
            mutator.remove_node(id);
            continue;
        }
        let label = mutator.create_element(
            QualName::new(None, ns!(html), local_name!("span")),
            vec![Attribute {
                name: QualName::new(None, ns!(), local_name!("style")),
                value: BLOCKED_STYLE.into(),
            }],
        );
        let text = mutator.create_text_node(&alt);
        mutator.append_children(label, &[text]);
        mutator.replace_node_with(id, &[label]);
    }
}

struct Provider {
    allowed: HashSet<String>,
    wake: Sender<Command>,
}

impl NetProvider for Provider {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.to_string();
        if request.url.scheme() == "data" {
            handler.bytes(url.clone(), Bytes::from(data_url(&url)));
            return;
        }
        if !self.allowed.contains(&url) {
            handler.bytes(url, Bytes::new());
            return;
        }
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            let bytes = fetch_image(&url).unwrap_or_default();
            handler.bytes(url, Bytes::from(bytes));
            let _ = wake.send(Command::Loaded);
        });
    }
}

fn data_url(url: &str) -> Vec<u8> {
    let Some((meta, data)) = url
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(','))
    else {
        return Vec::new();
    };
    if meta.ends_with(";base64") {
        STANDARD.decode(data.trim()).unwrap_or_default()
    } else {
        data.as_bytes().to_vec()
    }
}

fn fetch_image(url: &str) -> Option<Vec<u8>> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(IMAGE_TIMEOUT))
        .max_redirects(5)
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .header("User-Agent", "Mozilla/5.0")
        .call()
        .ok()?;
    let image = response
        .headers()
        .get("content-type")
        .and_then(|kind| kind.to_str().ok())
        .is_some_and(|kind| kind.trim_start().to_ascii_lowercase().starts_with("image/"));
    if !image {
        return None;
    }
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(IMAGE_LIMIT)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::{data_url, remote};

    #[test]
    fn reads_data_urls_and_tells_remote_sources_apart() {
        assert_eq!(data_url("data:image/png;base64,aGk="), b"hi");
        assert_eq!(data_url("data:text/plain,hi"), b"hi");
        assert!(remote("https://example.com/a.png"));
        assert!(remote("//tracker.example/p.gif"));
        assert!(!remote("data:image/png;base64,aGk="));
        assert!(!remote(""));
    }
}
