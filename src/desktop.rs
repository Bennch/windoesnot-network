use gtk4 as gtk;
use libadwaita as adw;
use adw::prelude::*;
// WinDoesNot desktop integration: accent colour, light/dark mode and UI
// language follow the shared desktop preferences
// (`~/.config/quickshell/default/settings.txt`) when they exist.
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::OnceLock;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

pub const DEFAULT_ACCENT: &str = "#fe6f2d";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    De,
    En,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DesktopPrefs {
    /// Validated `#rrggbb` accent colour.
    pub accent: Option<String>,
    pub dark: Option<bool>,
    pub language: Option<Language>,
    /// Corner radius in px, same setting Settings/Store use everywhere else.
    pub radius: Option<u32>,
}

pub fn settings_path() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    config.join("quickshell/default/settings.txt")
}

pub fn read_prefs() -> DesktopPrefs {
    std::fs::read_to_string(settings_path())
        .map(|text| parse_prefs(&text))
        .unwrap_or_default()
}

fn parse_prefs(text: &str) -> DesktopPrefs {
    let mut prefs = DesktopPrefs::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "accentColor" if is_hex_colour(value) => prefs.accent = Some(value.to_ascii_lowercase()),
            "darkMode" => prefs.dark = Some(value != "false"),
            "uiLanguage" => prefs.language = language_from(value),
            "borderRadius" => prefs.radius = value.parse().ok(),
            _ => {}
        }
    }
    prefs
}

fn is_hex_colour(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn language_from(value: &str) -> Option<Language> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() || value == "c" || value == "posix" {
        None
    } else if value.starts_with("de") {
        Some(Language::De)
    } else {
        Some(Language::En)
    }
}

fn system_language() -> Language {
    // LANGUAGE may list several languages ("de:en"); the first one wins.
    for var in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(var) {
            if let Some(language) = language_from(value.split(':').next().unwrap_or("")) {
                return language;
            }
        }
    }
    Language::En
}

/// Interface language for this run: the WinDoesNot setting, else the system locale.
/// Labels are built once, so a language change applies after restarting the store.
pub fn language() -> Language {
    static LANGUAGE: OnceLock<Language> = OnceLock::new();
    *LANGUAGE.get_or_init(|| read_prefs().language.unwrap_or_else(system_language))
}

pub fn accent() -> String {
    read_prefs().accent.unwrap_or_else(|| DEFAULT_ACCENT.to_string())
}

pub fn accent_rgb(accent: &str) -> (u8, u8, u8) {
    let channel = |i: usize| u8::from_str_radix(&accent[i..i + 2], 16).unwrap_or(0);
    (channel(1), channel(3), channel(5))
}

/// Readable text colour on top of the accent (WCAG relative luminance).
pub fn accent_foreground(accent: &str) -> &'static str {
    let (r, g, b) = accent_rgb(accent);
    let linear = |c: u8| {
        let c = c as f64 / 255.0;
        if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let luminance = 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
    if luminance > 0.45 { "#17120e" } else { "#ffffff" }
}

thread_local! {
    static MONITOR: RefCell<Option<gio::FileMonitor>> = RefCell::new(None);
    static PENDING: RefCell<Option<glib::SourceId>> = RefCell::new(None);
}

/// Calls `on_change` (debounced) whenever the desktop preferences file changes.
/// The directory is watched because the desktop may replace the file on save.
pub fn watch<F: Fn() + 'static>(on_change: F) {
    let path = settings_path();
    let Some(directory) = path.parent() else {
        return;
    };
    let Ok(monitor) = gio::File::for_path(directory)
        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
    else {
        return;
    };
    let on_change = std::rc::Rc::new(on_change);
    monitor.connect_changed(move |_, file, other, _| {
        let touches_settings = [Some(file), other]
            .into_iter()
            .flatten()
            .any(|candidate| candidate.basename().as_deref() == Some(std::path::Path::new("settings.txt")));
        if !touches_settings {
            return;
        }
        let on_change = on_change.clone();
        PENDING.with(|pending| {
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let id = glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                PENDING.with(|pending| pending.borrow_mut().take());
                on_change();
            });
            *pending.borrow_mut() = Some(id);
        });
    });
    MONITOR.with(|slot| *slot.borrow_mut() = Some(monitor));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_desktop_preferences() {
        let prefs = parse_prefs("accentColor=#3A86FF\ndarkMode=false\nuiLanguage=de\nother=x\n");
        assert_eq!(prefs.accent.as_deref(), Some("#3a86ff"));
        assert_eq!(prefs.dark, Some(false));
        assert_eq!(prefs.language, Some(Language::De));
        assert_eq!(parse_prefs("accentColor=red;}\n").accent, None);
        assert_eq!(parse_prefs("uiLanguage=en\n").language, Some(Language::En));
    }

    #[test]
    fn foreground_stays_readable() {
        assert_eq!(accent_foreground("#fe6f2d"), "#ffffff");
        assert_eq!(accent_foreground("#ffe14d"), "#17120e");
        assert_eq!(accent_rgb("#fe6f2d"), (254, 111, 45));
    }
}


pub fn sync_color_scheme() {
    adw::StyleManager::default().set_color_scheme(match read_prefs().dark {
        Some(true) => adw::ColorScheme::ForceDark,
        Some(false) => adw::ColorScheme::ForceLight,
        None => adw::ColorScheme::Default,
    });
}

/// Shared WinDoesNot colour and control contract (same palette as Store/Settings).
pub fn setup() {
    let provider = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(&gtk::gdk::Display::default().expect("display"), &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 50);
    let refresh = move || {
        let prefs = read_prefs();
        if std::env::var_os("WINDOESNOT_STYLE_DEBUG").is_some() { eprintln!("WDN_STYLE {:?} {:?}", settings_path(), prefs); }
        let manager = adw::StyleManager::default();
        manager.set_color_scheme(match prefs.dark {
            Some(true) => adw::ColorScheme::ForceDark,
            Some(false) => adw::ColorScheme::ForceLight,
            None => adw::ColorScheme::Default,
        });
        let (bg,sidebar,card,border,fg,selected) = if manager.is_dark() {
            ("#202020","#161616","#171717","#3a3a3a","#ffffff","#2a2a2a")
        } else { ("#f2f2f2","#e6e6e6","#f5f5f5","#d0d0d0","#1a1a1a","#c8c8c8") };
        let accent = prefs.accent.unwrap_or_else(|| DEFAULT_ACCENT.into());
        // Same default and same value everywhere as Settings' own borderRadius
        // (Settings.qml: `property int borderRadius: 10`, applied uniformly to
        // cards, rows and buttons there too).
        let radius = prefs.radius.unwrap_or(10);
        let css = format!(r#"
@define-color window_bg_color {bg}; @define-color window_fg_color {fg};
@define-color view_bg_color {bg}; @define-color view_fg_color {fg};
@define-color headerbar_bg_color {bg}; @define-color headerbar_fg_color {fg};
@define-color sidebar_bg_color {sidebar}; @define-color sidebar_fg_color {fg};
@define-color card_bg_color {card}; @define-color accent_bg_color {accent};
@define-color accent_color {accent}; @define-color accent_fg_color {accent_fg};
:root {{ --window-bg-color:{bg}; --window-fg-color:{fg}; --view-bg-color:{bg}; --view-fg-color:{fg};
 --headerbar-bg-color:{bg}; --headerbar-fg-color:{fg}; --sidebar-bg-color:{sidebar}; --sidebar-fg-color:{fg};
 --card-bg-color:{card}; --accent-bg-color:{accent}; --accent-color:{accent}; --accent-fg-color:{accent_fg}; }}
window, .background {{ background-color:{bg}; color:{fg}; font-family:Sans; font-size:14px; }}
headerbar {{ background:{bg}; box-shadow:none; border-bottom:1px solid {border}; }}
.wdn-sidebar, .sidebar {{ background:{sidebar}; }}
.wdn-brand {{ padding:28px 16px 24px; }}
.wdn-brand label {{ font-size:18px; font-weight:700; }}
.wdn-navigation {{ background:transparent; padding:0 8px; }}
.wdn-navigation row {{ min-height:44px; margin:3px 0; border-radius:{radius}px; padding:0 14px; }}
.wdn-navigation row:selected {{ background:{selected}; color:{fg}; box-shadow:inset 3px 0 {accent}; }}
.boxed-list {{ border-radius:{radius}px; border:1px solid {border}; background:{card}; box-shadow:none; }}
.card {{ border-radius:{radius}px; background:{card}; border-color:{border}; box-shadow:none; }}
button {{ border-radius:{radius}px; box-shadow:none; }}
button.suggested-action {{ background:{accent}; color:{accent_fg}; }}
"#, accent_fg=accent_foreground(&accent));
        provider.load_from_data(&css);
    };
    refresh();
    watch(refresh);
}

pub fn brand(title: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("wdn-brand");
    let picture = gtk::Picture::new();
    picture.set_size_request(118, 30);
    picture.set_can_shrink(true);
    picture.set_halign(gtk::Align::Start);
    picture.set_valign(gtk::Align::Center);
    let manager = adw::StyleManager::default();
    let update = |picture: &gtk::Picture, dark| {
        let bytes: &'static [u8] = if dark { include_bytes!("../assets/brand/windoesnot-logo-hell.png") }
            else { include_bytes!("../assets/brand/windoesnot-logo-dunkel.png") };
        if let Ok(pixbuf) = gtk::gdk_pixbuf::Pixbuf::from_read(std::io::Cursor::new(bytes)) {
            let height = (pixbuf.height() * 118 / pixbuf.width()).max(1);
            if let Some(scaled) = pixbuf.scale_simple(118, height, gtk::gdk_pixbuf::InterpType::Bilinear) {
                picture.set_paintable(Some(&gtk::gdk::Texture::for_pixbuf(&scaled)));
            }
        }
    };
    update(&picture, manager.is_dark());
    let weak = picture.downgrade();
    manager.connect_dark_notify(move |manager| { if let Some(picture)=weak.upgrade() { update(&picture,manager.is_dark()); } });
    row.append(&picture);
    let label = gtk::Label::new(Some(title));
    row.append(&label);
    row
}

pub fn tr(english: &'static str) -> &'static str {
    if language() != Language::De { return english; }
    match english {
        "Bluetooth unavailable" => "Bluetooth nicht verfügbar",
        "No Bluetooth adapter or service is available. Check that your adapter is connected and Bluetooth is enabled." => "Kein Bluetooth-Adapter oder Dienst verfügbar. Prüfe, ob dein Adapter angeschlossen und Bluetooth eingeschaltet ist.",
        "Wi-Fi" => "WLAN", "Network" => "Netzwerk", "Devices" => "Geräte", "Profiles" => "Profile",
        "Settings" => "Einstellungen", "About" => "Über", "Appearance" => "Darstellung",
        "System" => "System", "Light" => "Hell", "Dark" => "Dunkel", "Cancel" => "Abbrechen",
        "Close" => "Schließen", "Apply" => "Übernehmen", "Apply Changes" => "Änderungen übernehmen",
        "Save" => "Speichern", "Delete" => "Löschen", "Remove" => "Entfernen", "Connect" => "Verbinden",
        "Disconnect" => "Trennen", "Connected" => "Verbunden", "Disconnected" => "Nicht verbunden",
        "Connecting…" => "Verbindung wird hergestellt …", "Connecting..." => "Verbindung wird hergestellt …",
        "Checking status…" => "Status wird geprüft …", "Scanning..." => "Suche läuft …", "Scan" => "Suchen",
        "Password" => "Passwort", "Show Password" => "Passwort anzeigen", "Refresh" => "Aktualisieren",
        "No Networks Available" => "Keine Netzwerke verfügbar", "No networks found" => "Keine Netzwerke gefunden",
        "Available Networks" => "Verfügbare Netzwerke", "Saved Networks" => "Gespeicherte Netzwerke",
        "Known Networks" => "Bekannte Netzwerke", "Hidden Network" => "Verborgenes Netzwerk",
        "Connect to Hidden Network" => "Mit verborgenem Netzwerk verbinden", "Forget" => "Vergessen",
        "General Bluetooth Settings" => "Allgemeine Bluetooth-Einstellungen", "Bluetooth Settings" => "Bluetooth-Einstellungen",
        "All the devices you've connected to" => "Bereits gekoppelte Geräte", "No devices in range" => "Keine Geräte in Reichweite",
        "Device" => "Gerät", "Adapter" => "Adapter", "Adapters" => "Adapter", "Pair" => "Koppeln",
        "Paired" => "Gekoppelt", "Trust" => "Vertrauen", "Trusted" => "Vertrauenswürdig", "Block" => "Blockieren",
        "Blocked" => "Blockiert", "Unblock" => "Freigeben", "Rename" => "Umbenennen", "Send File" => "Datei senden",
        "File Transfers" => "Dateiübertragungen", "Received Files" => "Empfangene Dateien", "Discoverable" => "Sichtbar",
        "Discovering" => "Suche läuft", "More Information" => "Weitere Informationen", "Battery" => "Akku",
        "Connection status" => "Verbindungsstatus", "Settings & About" => "Einstellungen und Informationen",
        "Hotspot active" => "Hotspot aktiv", "Behavior" => "Verhalten", "Modules" => "Bereiche",
        "Reset" => "Zurücksetzen", "No network hardware detected. Check your hardware or restart NetworkManager." =>
            "Keine Netzwerkgeräte erkannt. Prüfe die Hardware und den NetworkManager-Dienst.",
        "(Highly insecure! Debug only)" => "Nur zur Fehlersuche – unsicher!",
        "0 clients (approximate)" => "Ungefähr 0 Geräte",
        "Activate" => "Aktivieren",
        "Activate profile" => "Profil aktivieren",
        "Active profile" => "Aktives Profil",
        "Add module" => "Bereich hinzufügen",
        "Add rule" => "Regel hinzufügen",
        "Addresses" => "Adressen",
        "Advanced Hotspot Controls" => "Erweiterte Hotspot-Einstellungen",
        "All" => "Alle",
        "Allow or block devices by MAC address" => "Geräte anhand ihrer MAC-Adresse erlauben oder blockieren",
        "Allowed IPs" => "Erlaubte IP-Adressen",
        "Always show connection details" => "Verbindungsdetails immer anzeigen",
        "Applies to time and upload/download quotas for device rules" => "Gilt für Zeit- und Datenkontingente der Geräteregeln",
        "Apply to active connection" => "Auf aktive Verbindung anwenden",
        "Assigned connections" => "Zugeordnete Verbindungen",
        "Auto" => "Automatisch",
        "Auto refresh networks" => "Netzwerke automatisch aktualisieren",
        "Auto-connect" => "Automatisch verbinden",
        "Automatically connect to this network when available" => "Automatisch mit diesem Netzwerk verbinden, wenn es verfügbar ist",
        "Back" => "Zurück",
        "Block this device" => "Dieses Gerät blockieren",
        "Blocked sites" => "Gesperrte Webseiten",
        "Bring back the safe default modules and order" => "Die Standardbereiche und ihre Reihenfolge wiederherstellen",
        "Channel" => "Kanal",
        "Checking for modems..." => "Modems werden gesucht …",
        "Checking for tc and nftables..." => "Unterstützung für Datenlimits wird geprüft …",
        "Choose which modules are shown in the top navigation" => "Wähle die sichtbaren Bereiche",
        "Clear" => "Leeren",
        "Clear all changes and restore defaults" => "Alle Änderungen zurücksetzen und Standardwerte wiederherstellen",
        "Configuration" => "Konfiguration",
        "Connect an ethernet cable or create a wired profile" => "Schließe ein Netzwerkkabel an oder erstelle ein Ethernet-Profil",
        "Connect automatically" => "Automatisch verbinden",
        "Connect to a hidden Wi-Fi network" => "Mit einem verborgenen WLAN verbinden",
        "Connected Devices" => "Verbundene Geräte",
        "Connected time limit" => "Maximale Verbindungsdauer",
        "Connected time quota" => "Zeitkontingent",
        "Connection" => "Verbindung",
        "Connection Details" => "Verbindungsdetails",
        "Connection details" => "Verbindungsdetails",
        "Connection name" => "Verbindungsname",
        "Connection type" => "Verbindungstyp",
        "Copy" => "Kopieren",
        "Copy IP" => "IP-Adresse kopieren",
        "Copy MAC" => "MAC-Adresse kopieren",
        "Copy password" => "Passwort kopieren",
        "Create VPN" => "VPN erstellen",
        "Create a WireGuard VPN or import an OpenVPN/WireGuard profile" => "Erstelle ein WireGuard-VPN oder importiere ein OpenVPN-/WireGuard-Profil",
        "Create a profile to manage auto-connect behavior" => "Erstelle ein Profil für automatische Verbindungen",
        "Custom Band" => "Benutzerdefiniertes Frequenzband",
        "Custom DNS" => "Eigene DNS-Server",
        "DNS servers" => "DNS-Server",
        "Default layout" => "Standardanordnung",
        "Delete VPN" => "VPN löschen",
        "Delete profile" => "Profil löschen",
        "Details" => "Details",
        "Device details" => "Gerätedetails",
        "Device name" => "Gerätename",
        "Devices will appear here when they join your hotspot" => "Mit dem Hotspot verbundene Geräte erscheinen hier",
        "Download limit" => "Downloadlimit",
        "Download quota" => "Downloadkontingent",
        "Download speed limit" => "Maximale Downloadgeschwindigkeit",
        "Drag to reorder" => "Zum Umsortieren ziehen",
        "Drop hotspot traffic for this MAC address" => "Hotspot-Datenverkehr dieser MAC-Adresse blockieren",
        "Edit" => "Bearbeiten",
        "Edit VPN" => "VPN bearbeiten",
        "Edit profile" => "Profil bearbeiten",
        "Edit rules" => "Regeln bearbeiten",
        "Endpoint" => "Endpunkt",
        "Ethernet is off" => "Ethernet ist ausgeschaltet",
        "Filter assignable connections" => "Zuordenbare Verbindungen filtern",
        "Forget Network" => "Netzwerk vergessen",
        "Frequency Band" => "Frequenzband",
        "Generate" => "Erzeugen",
        "Generate password" => "Passwort erzeugen",
        "Hide" => "Verbergen",
        "Hotspot Password" => "Hotspot-Passwort",
        "Hotspot is active" => "Hotspot ist aktiv",
        "Hotspot is off" => "Hotspot ist ausgeschaltet",
        "Hotspot password storage" => "Speicherung des Hotspot-Passworts",
        "Hotspot quota reset" => "Hotspot-Kontingente zurücksetzen",
        "IP address" => "IP-Adresse",
        "Immediately drops hotspot traffic for this MAC" => "Blockiert sofort den Hotspot-Datenverkehr dieser MAC-Adresse",
        "Import VPN" => "VPN importieren",
        "Interface name" => "Schnittstellenname",
        "Keep modules visible, but hide the text labels" => "Bereiche nur mit Symbolen anzeigen",
        "Keep the active network card expanded" => "Details des aktiven Netzwerks geöffnet lassen",
        "MAC address" => "MAC-Adresse",
        "MAC filtering" => "MAC-Filter",
        "MB in the current quota window, optional" => "MB im aktuellen Kontingentzeitraum, optional",
        "Manage" => "Verwalten",
        "Manage access" => "Zugriff verwalten",
        "Maximum connected devices" => "Maximale Anzahl verbundener Geräte",
        "Mobile Data" => "Mobile Daten",
        "Mod: right-click to edit" => "Bereich: Rechtsklick zum Bearbeiten",
        "Module order" => "Reihenfolge der Bereiche",
        "Module order is fixed across the app" => "Die Reihenfolge gilt für die gesamte Anwendung",
        "Network Details" => "Netzwerkdetails",
        "Network Interface" => "Netzwerkschnittstelle",
        "Network Name (SSID)" => "Netzwerkname (SSID)",
        "Network details" => "Netzwerkdetails",
        "Network won't be visible in WiFi lists" => "Das Netzwerk wird in WLAN-Listen nicht angezeigt",
        "Networks" => "Netzwerke",
        "New profile" => "Neues Profil",
        "No Networks Found" => "Keine Netzwerke gefunden",
        "No Wi-Fi adapter found" => "Kein WLAN-Adapter gefunden",
        "No Wired Connections" => "Keine Ethernet-Verbindungen",
        "No device-specific hotspot rules" => "Keine gerätespezifischen Hotspot-Regeln",
        "No devices found" => "Keine Geräte gefunden",
        "No profile sets" => "Keine Profilgruppen",
        "No supported VPN connections" => "Keine unterstützten VPN-Verbindungen",
        "Not connected" => "Nicht verbunden",
        "Not generated" => "Nicht erzeugt",
        "Open System Settings" => "Systemeinstellungen öffnen",
        "Optional" => "Optional",
        "Optional. Extra devices are blocked on the next policy refresh." => "Optional. Weitere Geräte werden bei der nächsten Regelaktualisierung blockiert.",
        "Other Networks" => "Weitere Netzwerke",
        "Password length" => "Passwortlänge",
        "Password strength" => "Passwortstärke",
        "Peer public key" => "Öffentlicher Schlüssel der Gegenstelle",
        "Per-device rules" => "Regeln pro Gerät",
        "Persistent keepalive" => "Regelmäßige Verbindungserhaltung",
        "Preshared key" => "Gemeinsamer geheimer Schlüssel",
        "Private key" => "Privater Schlüssel",
        "Profile Sets" => "Profilgruppen",
        "Profile name" => "Profilname",
        "Refresh devices" => "Geräte aktualisieren",
        "Refresh networks" => "Netzwerke aktualisieren",
        "Refresh profiles and VPN connections" => "Profile und VPN-Verbindungen aktualisieren",
        "Refresh wired connections" => "Ethernet-Verbindungen aktualisieren",
        "Requires sudo authentication" => "Benötigt Administratorrechte",
        "Rescan nearby networks every 10 seconds" => "Alle zehn Sekunden nach Netzwerken suchen",
        "Reset modules to defaults" => "Bereiche auf Standardwerte zurücksetzen",
        "Reset settings" => "Einstellungen zurücksetzen",
        "Reset to defaults" => "Auf Standardwerte zurücksetzen",
        "Restore" => "Wiederherstellen",
        "Restore top navigation" => "Navigation wiederherstellen",
        "Reveal" => "Anzeigen",
        "Reveal password" => "Passwort anzeigen",
        "Reveal saved password" => "Gespeichertes Passwort anzeigen",
        "Save hotspot changes and apply them if the hotspot is active" => "Hotspot-Änderungen speichern und bei aktivem Hotspot anwenden",
        "Saved" => "Gespeichert",
        "Scan this QR code to connect to the network" => "Scanne diesen QR-Code, um dich mit dem Netzwerk zu verbinden",
        "Search domains" => "Suchdomänen",
        "Seconds, optional" => "Sekunden, optional",
        "Security" => "Sicherheit",
        "Server / remote" => "Server / Gegenstelle",
        "Show QR Code" => "QR-Code anzeigen",
        "Show QR Code to Connect" => "QR-Code zum Verbinden anzeigen",
        "Show QR code for this hotspot" => "QR-Code für diesen Hotspot anzeigen",
        "Show the hotspot password in plain text" => "Das Hotspot-Passwort als Klartext anzeigen",
        "Signal" => "Signal",
        "Speed: —" => "Geschwindigkeit: —",
        "Status unavailable" => "Status nicht verfügbar",
        "System support" => "Systemunterstützung",
        "System, light, or dark" => "System, hell oder dunkel",
        "Temporary Guest Password" => "Temporäres Gastpasswort",
        "Turn On Wi-Fi" => "WLAN einschalten",
        "Turn on Wi-Fi or refresh to scan for networks" => "Schalte WLAN ein oder aktualisiere die Netzwerksuche",
        "Turn radio on" => "Funk einschalten",
        "Unavailable" => "Nicht verfügbar",
        "Upload limit" => "Uploadlimit",
        "Upload quota" => "Uploadkontingent",
        "Upload speed limit" => "Maximale Uploadgeschwindigkeit",
        "Use Ethernet" => "Ethernet verwenden",
        "Use Wi-Fi" => "WLAN verwenden",
        "Use icons only in top navigation" => "Nur Symbole in der Navigation anzeigen",
        "Username" => "Benutzername",
        "VPN Connections" => "VPN-Verbindungen",
        "Waiting for devices to connect..." => "Warte auf Geräteverbindungen …",
        "Waiting for devices to connect…" => "Warte auf Geräteverbindungen …",
        "Weak" => "Schwach",
        "Wi-Fi Password" => "WLAN-Passwort",
        "Wi-Fi is Off" => "WLAN ist ausgeschaltet",
        "Wi-Fi is off" => "WLAN ist ausgeschaltet",
        "Wi-Fi off" => "WLAN aus",
        "estimated" => "geschätzt",
        "kbit/s, optional" => "kbit/s, optional",
        "minutes in the current quota window, optional" => "Minuten im aktuellen Kontingentzeitraum, optional",
        "sudo password" => "Administratorpasswort",
        "tc and nftables are available" => "Unterstützung für Datenlimits verfügbar",
        "Network change notifications unavailable; using periodic refresh" => "Netzwerkänderungen werden regelmäßig abgefragt.",
        "Unable to retrieve data from NetworkManager – check your connection" => "Netzwerkdaten nicht verfügbar. Prüfe, ob der NetworkManager-Dienst läuft.",
        _ => english,
    }
}
