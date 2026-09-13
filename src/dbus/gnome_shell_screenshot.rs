use std::collections::HashMap;
use std::path::PathBuf;

use neori_ipc::PickedColor;
use zbus::zvariant::OwnedValue;
use zbus::{fdo, interface, zvariant};

use super::{request_name, Start};

pub struct Screenshot {
    to_neori: calloop::channel::Sender<ScreenshotToNeori>,
    from_neori: async_channel::Receiver<NeoriToScreenshot>,
}

pub enum ScreenshotToNeori {
    TakeScreenshot { include_cursor: bool },
    PickColor(async_channel::Sender<Option<PickedColor>>),
}

pub enum NeoriToScreenshot {
    ScreenshotResult(Option<PathBuf>),
}

#[interface(name = "org.gnome.Shell.Screenshot")]
impl Screenshot {
    async fn screenshot(
        &self,
        include_cursor: bool,
        _flash: bool,
        _filename: PathBuf,
    ) -> fdo::Result<(bool, PathBuf)> {
        if let Err(err) = self
            .to_neori
            .send(ScreenshotToNeori::TakeScreenshot { include_cursor })
        {
            warn!("error sending message to neori: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }

        let filename = match self.from_neori.recv().await {
            Ok(NeoriToScreenshot::ScreenshotResult(Some(filename))) => filename,
            Ok(NeoriToScreenshot::ScreenshotResult(None)) => {
                return Err(fdo::Error::Failed("internal error".to_owned()));
            }
            Err(err) => {
                warn!("error receiving message from neori: {err:?}");
                return Err(fdo::Error::Failed("internal error".to_owned()));
            }
        };

        Ok((true, filename))
    }

    async fn pick_color(&self) -> fdo::Result<HashMap<String, OwnedValue>> {
        let (tx, rx) = async_channel::bounded(1);
        if let Err(err) = self.to_neori.send(ScreenshotToNeori::PickColor(tx)) {
            warn!("error sending pick color message to neori: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }

        let color = match rx.recv().await {
            Ok(Some(color)) => color,
            Ok(None) => {
                return Err(fdo::Error::Failed("no color picked".to_owned()));
            }
            Err(err) => {
                warn!("error receiving message from neori: {err:?}");
                return Err(fdo::Error::Failed("internal error".to_owned()));
            }
        };

        let mut result = HashMap::new();
        let [r, g, b] = color.rgb;
        result.insert(
            "color".to_string(),
            zvariant::OwnedValue::try_from(zvariant::Value::from((r, g, b))).unwrap(),
        );

        Ok(result)
    }
}

impl Screenshot {
    pub fn new(
        to_neori: calloop::channel::Sender<ScreenshotToNeori>,
        from_neori: async_channel::Receiver<NeoriToScreenshot>,
    ) -> Self {
        Self {
            to_neori,
            from_neori,
        }
    }
}

impl Start for Screenshot {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection> {
        let conn = zbus::blocking::Connection::session()?;
        conn.object_server()
            .at("/org/gnome/Shell/Screenshot", self)?;
        request_name(&conn, "org.gnome.Shell.Screenshot")?;

        Ok(conn)
    }
}
