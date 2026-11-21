//! Other lighting and fan software that grabs the same devices. The app and the service share this list.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Area {
    Lighting,
    Fans,
    Both,
}

#[derive(Debug, PartialEq, Eq)]
pub struct App {
    pub id: &'static str,
    pub name: &'static str,
    pub area: Area,
    /// Executable names, matched without case; a trailing `*` matches any ending.
    pub processes: &'static [&'static str],
    /// Exact Windows service names.
    pub services: &'static [&'static str],
}

pub const APPS: &[App] = &[
    App {
        id: "ghub",
        name: "Logitech G HUB",
        area: Area::Lighting,
        processes: &["lghub*"],
        services: &["LGHUBUpdaterService"],
    },
    App {
        id: "signalrgb",
        name: "SignalRGB",
        area: Area::Both,
        processes: &["SignalRgb*", "VortxEngine*"],
        services: &["SignalRgb.Service"],
    },
    App {
        id: "icue",
        name: "Corsair iCUE",
        area: Area::Both,
        processes: &["iCUE*", "Corsair.Service*", "CueLLAccessService*"],
        services: &[
            "CorsairService",
            "CorsairLLAService",
            "CorsairGamingAudioConfig",
            "CorsairDeviceListingService",
            "CorsairCpuIdService",
        ],
    },
    App {
        id: "armoury",
        name: "ASUS Armoury Crate",
        area: Area::Both,
        processes: &[
            "ArmouryCrate*",
            "ArmourySocketServer*",
            "LightingService.exe",
            "AsusCertService.exe",
            "AuraWallpaperService*",
        ],
        services: &[
            "ArmouryCrateService",
            "ArmouryCrateControlInterface",
            "LightingService",
            "AsusCertService",
            "AsusFanControlService",
        ],
    },
    App {
        id: "msi",
        name: "MSI Center",
        area: Area::Both,
        processes: &[
            "MSI.CentralServer*",
            "MSI Center*",
            "MysticLight*",
            "LEDKeeper*",
            "LightKeeperService*",
        ],
        services: &[
            "MSI_Central_Service",
            "Mystic_Light_Service",
            "LightKeeperService",
        ],
    },
    App {
        id: "steelseries",
        name: "SteelSeries GG",
        area: Area::Lighting,
        processes: &[
            "SteelSeriesGG*",
            "SteelSeriesEngine*",
            "SteelSeriesPrism*",
            "SteelSeriesSonar*",
        ],
        services: &["SteelSeriesGGUpdateServiceProxy"],
    },
    App {
        id: "razer",
        name: "Razer Synapse",
        area: Area::Lighting,
        processes: &[
            "Razer Synapse*",
            "RzSynapse*",
            "RazerCentral*",
            "Razer Central*",
            "RzChromaSDK*",
            "RzSDK*",
            "RazerAppEngine*",
        ],
        services: &[
            "Razer Synapse Service",
            "RzActionSvc",
            "Razer Chroma SDK Service",
            "Razer Chroma SDK Server",
            "Razer Game Manager Service",
        ],
    },
    App {
        id: "nzxt",
        name: "NZXT CAM",
        area: Area::Both,
        processes: &["NZXT CAM*", "NZXT.CAM*"],
        services: &[],
    },
    App {
        id: "gigabyte",
        name: "Gigabyte RGB Fusion",
        area: Area::Both,
        processes: &["RGBFusion*", "RGB Fusion*", "GigabyteControlCenter*"],
        services: &[],
    },
    App {
        id: "polychrome",
        name: "ASRock Polychrome RGB",
        area: Area::Lighting,
        processes: &["PolychromeRGB*", "Polychrome RGB*", "AsrPolychrome*"],
        services: &[],
    },
    App {
        id: "ttrgb",
        name: "Thermaltake TT RGB Plus",
        area: Area::Both,
        processes: &["TT RGB Plus*", "TTRGBPlus*", "TT_RGB_Plus*"],
        services: &[],
    },
    App {
        id: "masterplus",
        name: "Cooler Master MasterPlus",
        area: Area::Both,
        processes: &["MasterPlus*"],
        services: &[],
    },
    App {
        id: "ngenuity",
        name: "HyperX NGENUITY",
        area: Area::Lighting,
        processes: &["NGENUITY*", "HyperX NGENUITY*", "NGenuity2Helper*"],
        services: &[],
    },
    App {
        id: "swarm",
        name: "Roccat Swarm",
        area: Area::Lighting,
        processes: &["ROCCAT_Swarm*", "ROCCAT Swarm*"],
        services: &[],
    },
    App {
        id: "openrgb",
        name: "OpenRGB",
        area: Area::Lighting,
        processes: &["OpenRGB.exe"],
        services: &[],
    },
    App {
        id: "fancontrol",
        name: "Fan Control",
        area: Area::Fans,
        processes: &["FanControl.exe"],
        services: &[],
    },
];

/// Never closed, whatever a pattern says.
const PROTECTED: &[&str] = &[
    "foxden*",
    "svchost.exe",
    "explorer.exe",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "smss.exe",
    "system",
    "cargo.exe",
    "rustc.exe",
    "gcc.exe",
];

/// Case-insensitive; the pattern is an exact name or a prefix ending in `*`.
pub fn matches(pattern: &str, name: &str) -> bool {
    let (pattern, name) = (pattern.to_lowercase(), name.to_lowercase());
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

fn protected(name: &str) -> bool {
    PROTECTED.iter().any(|p| matches(p, name))
}

/// The listed app a process belongs to, if any.
pub fn app_of(process: &str) -> Option<&'static App> {
    if protected(process) {
        return None;
    }
    APPS.iter()
        .find(|app| app.processes.iter().any(|p| matches(p, process)))
}

/// What may be closed: only apps that clash with a module that is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scope {
    pub lighting: bool,
    pub fans: bool,
    /// FoxDen is using an OpenRGB server, so OpenRGB stays.
    pub keep_openrgb: bool,
}

impl Scope {
    pub fn covers(self, app: &App) -> bool {
        if self.kept_for(app).is_some() {
            return false;
        }
        match app.area {
            Area::Lighting => self.lighting,
            Area::Fans => self.fans,
            Area::Both => self.lighting || self.fans,
        }
    }

    /// The FoxDen feature that keeps this app running, if any.
    pub fn kept_for(self, app: &App) -> Option<&'static str> {
        match app.id {
            "openrgb" if self.keep_openrgb => Some("OpenRGB"),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub name: String,
}

/// The running processes to close, never `own` (the caller's own process id).
pub fn targets(running: &[Process], scope: Scope, own: u32) -> Vec<(&Process, &'static App)> {
    running
        .iter()
        .filter(|p| p.pid != own && p.pid > 4)
        .filter_map(|p| app_of(&p.name).map(|app| (p, app)))
        .filter(|(_, app)| scope.covers(app))
        .collect()
}

/// Names of the apps that were closed and of those that could not be.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Closed {
    pub closed: Vec<String>,
    pub left: Vec<String>,
}

impl Closed {
    /// Notes one process or service of `app`. An app with anything left over counts as left.
    pub fn add(&mut self, app: &App, ok: bool) {
        let name = app.name.to_string();
        if ok {
            if !self.closed.contains(&name) && !self.left.contains(&name) {
                self.closed.push(name);
            }
        } else {
            self.closed.retain(|n| *n != name);
            if !self.left.contains(&name) {
                self.left.push(name);
            }
        }
    }

    /// Adds what another closer did; what it closed is no longer left.
    pub fn merge(&mut self, other: Closed) {
        for name in other.closed {
            self.left.retain(|n| *n != name);
            if !self.closed.contains(&name) {
                self.closed.push(name);
            }
        }
        for name in other.left {
            if !self.closed.contains(&name) && !self.left.contains(&name) {
                self.left.push(name);
            }
        }
    }
}

/// The listed services to stop.
pub fn services(scope: Scope) -> impl Iterator<Item = (&'static str, &'static App)> {
    APPS.iter()
        .filter(move |app| scope.covers(app))
        .flat_map(|app| app.services.iter().map(move |s| (*s, app)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Scope = Scope {
        lighting: true,
        fans: true,
        keep_openrgb: false,
    };

    fn procs(names: &[&str]) -> Vec<Process> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| Process {
                pid: 100 + i as u32 * 4,
                name: n.to_string(),
            })
            .collect()
    }

    fn ids(found: &[(&Process, &App)]) -> Vec<&'static str> {
        found.iter().map(|(_, a)| a.id).collect()
    }

    #[test]
    fn names_match_without_case_and_by_prefix() {
        assert!(matches("lghub*", "LGHUB_agent.exe"));
        assert!(matches("FanControl.exe", "fancontrol.EXE"));
        assert!(!matches("FanControl.exe", "FanControl.exe.bak"));
        assert!(!matches("iCUE*", "xicue.exe"));
        let found = [
            ("lghub.exe", "ghub"),
            ("SignalRgbLauncher.exe", "signalrgb"),
            ("VortxEngine.exe", "signalrgb"),
            ("iCUE.exe", "icue"),
            ("ArmouryCrate.UserSessionHelper.exe", "armoury"),
            ("LEDKeeper2.exe", "msi"),
            ("SteelSeriesGG.exe", "steelseries"),
            ("Razer Synapse Service.exe", "razer"),
            ("NZXT CAM.exe", "nzxt"),
            ("OpenRGB.exe", "openrgb"),
            ("FanControl.exe", "fancontrol"),
        ];
        for (name, id) in found {
            assert_eq!(app_of(name).map(|a| a.id), Some(id), "{name}");
        }
    }

    #[test]
    fn system_and_own_processes_never_match() {
        for name in [
            "foxden.exe",
            "FoxDen.exe",
            "foxden-service.exe",
            "svchost.exe",
            "explorer.exe",
            "csrss.exe",
            "cargo.exe",
            "rustc.exe",
            "gcc.exe",
            "GCC.exe",
            "System",
            "code.exe",
        ] {
            assert_eq!(app_of(name), None, "{name}");
        }
        for app in APPS {
            for pattern in app.processes {
                assert!(pattern.len() > 4, "{pattern} is too broad");
            }
        }
    }

    #[test]
    fn only_apps_that_clash_with_a_module_that_is_on_are_closed() {
        let running = procs(&["FanControl.exe", "lghub.exe", "iCUE.exe", "notepad.exe"]);
        assert_eq!(
            ids(&targets(&running, ALL, 1)),
            ["fancontrol", "ghub", "icue"]
        );
        let lighting = Scope { fans: false, ..ALL };
        assert_eq!(ids(&targets(&running, lighting, 1)), ["ghub", "icue"]);
        let fans = Scope {
            lighting: false,
            ..ALL
        };
        assert_eq!(ids(&targets(&running, fans, 1)), ["fancontrol", "icue"]);
        assert!(targets(&running, Scope::default(), 1).is_empty());
    }

    #[test]
    fn openrgb_stays_while_foxden_uses_it() {
        let running = procs(&["OpenRGB.exe"]);
        assert_eq!(ids(&targets(&running, ALL, 1)), ["openrgb"]);
        let keep = Scope {
            keep_openrgb: true,
            ..ALL
        };
        assert!(targets(&running, keep, 1).is_empty());
    }

    #[test]
    fn the_caller_and_system_ids_are_left_alone() {
        let mut running = procs(&["lghub.exe"]);
        assert!(targets(&running, ALL, running[0].pid).is_empty());
        running[0].pid = 4;
        assert!(targets(&running, ALL, 1).is_empty());
    }

    #[test]
    fn what_the_service_closed_is_no_longer_left() {
        let ghub = &APPS[0];
        let icue = &APPS[2];
        let mut app = Closed::default();
        app.add(ghub, true);
        app.add(icue, true);
        app.add(icue, false);
        assert_eq!(app.closed, ["Logitech G HUB"]);
        assert_eq!(app.left, ["Corsair iCUE"]);
        let mut service = Closed::default();
        service.add(icue, true);
        app.merge(service);
        assert_eq!(app.closed, ["Logitech G HUB", "Corsair iCUE"]);
        assert!(app.left.is_empty());
    }

    #[test]
    fn services_follow_the_scope() {
        let all: Vec<_> = services(ALL).map(|(s, _)| s).collect();
        assert!(all.contains(&"LGHUBUpdaterService"));
        assert!(all.contains(&"SignalRgb.Service"));
        assert!(all.contains(&"ArmouryCrateControlInterface"));
        let fans = Scope {
            lighting: false,
            ..ALL
        };
        let fans: Vec<_> = services(fans).map(|(s, _)| s).collect();
        assert!(!fans.contains(&"LGHUBUpdaterService"));
        assert!(fans.contains(&"CorsairService"));
        assert_eq!(services(Scope::default()).count(), 0);
    }
}
