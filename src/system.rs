// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Clock {
    TwelveHour,
    TwentyFourHour,
}

#[cfg(target_os = "linux")]
mod desktop {
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::OwnedValue;

    fn read(namespace: &str, key: &str) -> Option<OwnedValue> {
        let connection = Connection::session().ok()?;
        let settings = Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
        )
        .ok()?;
        settings.call("ReadOne", &(namespace, key)).ok()
    }

    pub fn dark() -> Option<bool> {
        let scheme = u32::try_from(read("org.freedesktop.appearance", "color-scheme")?).ok()?;
        Some(scheme == 1)
    }

    pub fn twelve_hour() -> Option<bool> {
        let format = String::try_from(read("org.gnome.desktop.interface", "clock-format")?).ok()?;
        Some(format == "12h")
    }
}

#[cfg(target_os = "linux")]
pub fn dark() -> Option<bool> {
    desktop::dark()
}

#[cfg(not(target_os = "linux"))]
pub fn dark() -> Option<bool> {
    None
}

pub fn clock() -> Clock {
    #[cfg(target_os = "linux")]
    if desktop::twelve_hour() == Some(true) {
        return Clock::TwelveHour;
    }
    Clock::TwentyFourHour
}
