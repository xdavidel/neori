use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use neori_config::Config;
use ordered_float::NotNan;
use pangocairo::cairo::{self, ImageSurface};
use pangocairo::pango::FontDescription;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::output::Output;
use smithay::reexports::gbm::Format as Fourcc;
use smithay::utils::{Point, Transform};

use crate::animation::{Animation, Clock};
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::NeoriRenderer;
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};
use crate::utils::{output_size, to_physical_precise_round};

const PADDING: i32 = 8;
const FONT: &str = "sans 14px";
const BORDER: i32 = 4;

pub struct ConfigErrorNotification {
    state: State,
    buffers: RefCell<HashMap<NotNan<f64>, Option<TextureBuffer<GlesTexture>>>>,

    // What the notification says.
    message: Message,

    // Where Lua errors are written, for the Lua error message.
    lua_log: Option<PathBuf>,

    clock: Clock,
    config: Rc<RefCell<Config>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Message {
    /// The config file failed to parse.
    ConfigError,
    /// A Lua callback failed while running.
    LuaError,
    /// A default config file was created at this path.
    Created(PathBuf),
}

enum State {
    Hidden,
    Showing(Animation),
    Shown(Duration),
    Hiding(Animation),
}

impl ConfigErrorNotification {
    pub fn new(clock: Clock, config: Rc<RefCell<Config>>) -> Self {
        Self {
            state: State::Hidden,
            buffers: RefCell::new(HashMap::new()),
            message: Message::ConfigError,
            lua_log: None,
            clock,
            config,
        }
    }

    fn animation(&self, from: f64, to: f64) -> Animation {
        let c = self.config.borrow();
        Animation::new(
            self.clock.clone(),
            from,
            to,
            0.,
            c.animations.config_notification_open_close.0,
        )
    }

    /// Sets the file that Lua errors are written to, which the Lua error message names.
    pub fn set_lua_log(&mut self, path: PathBuf) {
        self.lua_log = Some(path);
        if self.message == Message::LuaError {
            self.buffers.borrow_mut().clear();
        }
    }

    fn set_message(&mut self, message: Message) {
        if self.message != message {
            self.message = message;
            self.buffers.borrow_mut().clear();
        }
    }

    pub fn show_created(&mut self, created_path: &Path) {
        self.set_message(Message::Created(created_path.to_owned()));
        self.state = State::Showing(self.animation(0., 1.));
    }

    /// Shows that the config file failed to parse.
    pub fn show(&mut self) {
        self.show_failure(Message::ConfigError);
    }

    /// Shows that a Lua callback failed while running. The config itself parsed, so this names
    /// the log with the error rather than suggesting `neori validate`.
    pub fn show_lua_error(&mut self) {
        self.show_failure(Message::LuaError);
    }

    fn show_failure(&mut self, message: Message) {
        if self.config.borrow().config_notification.disable_failed {
            return;
        }
        self.set_message(message);

        // Show from scratch even if already showing to bring attention.
        self.state = State::Showing(self.animation(0., 1.));
    }

    /// Whether it's showing, or about to show, that a Lua callback failed.
    #[cfg(test)]
    pub fn shows_lua_error(&self) -> bool {
        !matches!(self.state, State::Hidden) && self.message == Message::LuaError
    }

    pub fn hide(&mut self) {
        if matches!(self.state, State::Hidden) {
            return;
        }

        self.state = State::Hiding(self.animation(1., 0.));
    }

    pub fn advance_animations(&mut self) {
        match &mut self.state {
            State::Hidden => (),
            State::Showing(anim) => {
                if anim.is_done() {
                    let duration = if matches!(self.message, Message::Created(_)) {
                        // Make this quite a bit longer because it comes with a monitor modeset
                        // (can take a while) and an important hotkeys popup diverting the
                        // attention.
                        Duration::from_secs(8)
                    } else {
                        Duration::from_secs(4)
                    };
                    self.state = State::Shown(self.clock.now_unadjusted() + duration);
                }
            }
            State::Shown(deadline) => {
                if self.clock.now_unadjusted() >= *deadline {
                    self.hide();
                }
            }
            State::Hiding(anim) => {
                if anim.is_clamped_done() {
                    self.state = State::Hidden;
                }
            }
        }
    }

    pub fn are_animations_ongoing(&self) -> bool {
        !matches!(self.state, State::Hidden)
    }

    pub fn render<R: NeoriRenderer>(
        &self,
        renderer: &mut R,
        output: &Output,
    ) -> Option<PrimaryGpuTextureRenderElement> {
        if matches!(self.state, State::Hidden) {
            return None;
        }

        let scale = output.current_scale().fractional_scale();
        let output_size = output_size(output);
        let message = &self.message;
        let lua_log = self.lua_log.as_deref();

        let mut buffers = self.buffers.borrow_mut();
        let buffer = buffers
            .entry(NotNan::new(scale).unwrap())
            .or_insert_with(move || {
                render(renderer.as_gles_renderer(), scale, message, lua_log).ok()
            });
        let buffer = buffer.clone()?;

        let size = buffer.logical_size();
        let y_range = size.h + f64::from(PADDING) * 2.;

        let x = (output_size.w - size.w).max(0.) / 2.;
        let y = match &self.state {
            State::Hidden => unreachable!(),
            State::Showing(anim) | State::Hiding(anim) => -size.h + anim.value() * y_range,
            State::Shown(_) => f64::from(PADDING) * 2.,
        };

        let location = Point::from((x, y));
        let location = location.to_physical_precise_round(scale).to_logical(scale);

        let elem = TextureRenderElement::from_texture_buffer(
            buffer,
            location,
            1.,
            None,
            None,
            Kind::Unspecified,
        );
        Some(PrimaryGpuTextureRenderElement(elem))
    }
}

fn render(
    renderer: &mut GlesRenderer,
    scale: f64,
    message: &Message,
    lua_log: Option<&Path>,
) -> anyhow::Result<TextureBuffer<GlesTexture>> {
    let _span = tracy_client::span!("config_error_notification::render");

    let padding: i32 = to_physical_precise_round(scale, PADDING);

    let mut text = match message {
        Message::LuaError => lua_error_text(true, lua_log),
        _ => error_text(true),
    };
    let mut border_color = (1., 0.3, 0.3);
    if let Message::Created(path) = message {
        text = format!(
            "Created a default config file at \
             <span face='monospace' bgcolor='#000000'>{path:?}</span>",
        );
        border_color = (0.5, 1., 0.5);
    };

    let mut font = FontDescription::from_string(FONT);
    font.set_absolute_size(to_physical_precise_round(scale, font.size()));

    let surface = ImageSurface::create(cairo::Format::ARgb32, 0, 0)?;
    let cr = cairo::Context::new(&surface)?;
    let layout = pangocairo::functions::create_layout(&cr);
    layout.context().set_round_glyph_positions(false);
    layout.set_font_description(Some(&font));
    layout.set_markup(&text);

    let (mut width, mut height) = layout.pixel_size();
    width += padding * 2;
    height += padding * 2;

    let surface = ImageSurface::create(cairo::Format::ARgb32, width, height)?;
    let cr = cairo::Context::new(&surface)?;
    cr.set_source_rgb(0.1, 0.1, 0.1);
    cr.paint()?;

    cr.move_to(padding.into(), padding.into());
    let layout = pangocairo::functions::create_layout(&cr);
    layout.context().set_round_glyph_positions(false);
    layout.set_font_description(Some(&font));
    layout.set_markup(&text);

    cr.set_source_rgb(1., 1., 1.);
    pangocairo::functions::show_layout(&cr, &layout);

    cr.move_to(0., 0.);
    cr.line_to(width.into(), 0.);
    cr.line_to(width.into(), height.into());
    cr.line_to(0., height.into());
    cr.line_to(0., 0.);
    cr.set_source_rgb(border_color.0, border_color.1, border_color.2);
    // Keep the border width even to avoid blurry edges.
    cr.set_line_width((f64::from(BORDER) / 2. * scale).round() * 2.);
    cr.stroke()?;
    drop(cr);

    let data = surface.take_data().unwrap();
    let buffer = TextureBuffer::from_memory(
        renderer,
        &data,
        Fourcc::Argb8888,
        (width, height),
        false,
        scale,
        Transform::Normal,
        Vec::new(),
    )?;

    Ok(buffer)
}

pub fn error_text(markup: bool) -> String {
    let command = if markup {
        "<span face='monospace' bgcolor='#000000'>neori validate</span>"
    } else {
        "neori validate"
    };

    format!("Failed to parse the config file. Please run {command} to see the errors.")
}

/// The message shown when a Lua callback fails. The config parsed fine, so it names the log that
/// has the error instead of `neori validate`.
pub fn lua_error_text(markup: bool, lua_log: Option<&Path>) -> String {
    let Some(path) = lua_log else {
        return String::from("A Lua callback failed. The error is in neori's log.");
    };
    let path = match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) => match path.strip_prefix(&home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        },
        None => path.display().to_string(),
    };
    if markup {
        let path = pangocairo::glib::markup_escape_text(&path);
        format!(
            "A Lua callback failed. The error is in \
             <span face='monospace' bgcolor='#000000'>{path}</span>."
        )
    } else {
        format!("A Lua callback failed. The error is in {path}.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lua_error_text_names_the_log() {
        assert_eq!(
            lua_error_text(false, None),
            "A Lua callback failed. The error is in neori's log."
        );
        let path = Path::new("/var/log/neori <lua>.log");
        assert_eq!(
            lua_error_text(false, Some(path)),
            "A Lua callback failed. The error is in /var/log/neori <lua>.log."
        );
        // The path is escaped for Pango markup.
        assert!(lua_error_text(true, Some(path)).contains("neori &lt;lua&gt;.log"));
    }
}
