//! Which channel an app belongs on by default, from its executable name and folder.

use crate::Channel;

const CHAT: &[&str] = &[
    "discord",
    "discordptb",
    "discordcanary",
    "vesktop",
    "ts3client_win64",
    "ts3client_win32",
    "teamspeak",
    "mumble",
    "skype",
    "ms-teams",
    "teams",
    "zoom",
    "slack",
    "whatsapp",
    "telegram",
    "signal",
    "guilded",
    "element",
];

const MEDIA: &[&str] = &[
    "spotify",
    "chrome",
    "msedge",
    "firefox",
    "brave",
    "opera",
    "opera_gx",
    "vivaldi",
    "arc",
    "vlc",
    "mpv",
    "mpc-hc",
    "mpc-hc64",
    "mpc-be",
    "mpc-be64",
    "potplayer",
    "potplayer64",
    "potplayermini",
    "potplayermini64",
    "foobar2000",
    "musicbee",
    "aimp",
    "itunes",
    "applemusic",
    "tidal",
    "deezer",
    "plex",
    "plex htpc",
    "jellyfin",
    "jellyfin media player",
    "amazon music",
    "youtube music",
    "youtube music desktop app",
];

/// Launchers and their helpers live beside games but are not games.
const LAUNCHERS: &[&str] = &[
    "steam",
    "steamwebhelper",
    "epicgameslauncher",
    "epicwebhelper",
    "battle.net",
    "agent",
    "galaxyclient",
    "galaxyclient helper",
    "eadesktop",
    "easteamproxy",
    "ubisoftconnect",
    "upc",
    "riotclientservices",
    "riotclientux",
    "riotclientuxrender",
    "leagueclient",
    "leagueclientux",
    "leagueclientuxrender",
    "rockstarlauncher",
    "launcher",
    "socialclubhelper",
    "xboxpcapp",
    "gamebar",
    "unitycrashhandler64",
    "crashreportclient",
];

const GAMES: &[&str] = &[
    "fortniteclient-win64-shipping",
    "valorant",
    "valorant-win64-shipping",
    "cs2",
    "csgo",
    "league of legends",
    "dota2",
    "r5apex",
    "overwatch",
    "gta5",
    "gta5_enhanced",
    "rocketleague",
    "eldenring",
    "minecraft.windows",
    "wow",
    "diablo iv",
    "cod",
];

/// Game library folders, lowercase with forward slashes.
const LIBRARIES: &[&str] = &[
    "/steamapps/common/",
    "/epic games/",
    "/gog galaxy/games/",
    "/gog games/",
    "/xboxgames/",
    "/riot games/",
    "/ea games/",
    "/ubisoft game launcher/games/",
    "/rockstar games/",
    "/battle.net/",
    "/blizzard/",
    "/overwatch/",
    "/world of warcraft/",
    "/diablo iv/",
    "/call of duty/",
    "/hearthstone/",
    "/starcraft ii/",
];

/// The preset channel for an app, or `None` to leave it alone.
pub fn preset(exe: &str, path: &str) -> Option<Channel> {
    let name = exe.to_lowercase();
    let stem = name.strip_suffix(".exe").unwrap_or(&name);
    let path = path.to_lowercase().replace('\\', "/");
    if LAUNCHERS.contains(&stem) {
        return None;
    }
    if CHAT.contains(&stem) {
        return Some(Channel::ChatRender);
    }
    if MEDIA.contains(&stem) {
        return Some(Channel::Media);
    }
    let game = GAMES.contains(&stem)
        || stem == "javaw" && path.contains("minecraft")
        || LIBRARIES.iter().any(|l| path.contains(l))
        // Store games: WindowsApps also holds ordinary apps, so only a game's Content folder counts.
        || path.contains("/windowsapps/") && path.contains("/content/");
    (game && !path.contains("/epic games/launcher/")).then_some(Channel::Game)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_and_media_go_by_name() {
        assert_eq!(preset("Discord.exe", ""), Some(Channel::ChatRender));
        assert_eq!(preset("ts3client_win64.exe", ""), Some(Channel::ChatRender));
        assert_eq!(preset("ms-teams.exe", ""), Some(Channel::ChatRender));
        assert_eq!(preset("Spotify.exe", ""), Some(Channel::Media));
        assert_eq!(
            preset(
                "firefox.exe",
                r"C:\Program Files\Mozilla Firefox\firefox.exe"
            ),
            Some(Channel::Media)
        );
        assert_eq!(preset("Amazon Music.exe", ""), Some(Channel::Media));
        assert_eq!(preset("notepad.exe", r"C:\Windows\notepad.exe"), None);
    }

    #[test]
    fn games_go_by_library_folder_or_name() {
        let steam = r"D:\SteamLibrary\steamapps\common\Hades II\Hades2.exe";
        assert_eq!(preset("Hades2.exe", steam), Some(Channel::Game));
        assert_eq!(
            preset(
                "Witcher3.exe",
                r"C:\GOG Games\The Witcher 3\bin\x64\witcher3.exe"
            ),
            Some(Channel::Game)
        );
        assert_eq!(
            preset("Game.exe", r"C:\XboxGames\Starfield\Content\Starfield.exe"),
            Some(Channel::Game)
        );
        assert_eq!(preset("cs2.exe", ""), Some(Channel::Game));
        assert_eq!(
            preset("FortniteClient-Win64-Shipping.exe", ""),
            Some(Channel::Game)
        );
        assert_eq!(
            preset(
                "javaw.exe",
                r"C:\Users\x\AppData\Roaming\.minecraft\runtime\bin\javaw.exe"
            ),
            Some(Channel::Game)
        );
        assert_eq!(
            preset("javaw.exe", r"C:\Program Files\Java\bin\javaw.exe"),
            None
        );
        assert_eq!(
            preset(
                "App.exe",
                r"C:\Program Files\WindowsApps\Netflix_1.0\App.exe"
            ),
            None
        );
    }

    #[test]
    fn launchers_are_not_games() {
        let lib = r"C:\Program Files (x86)\Steam\steam.exe";
        assert_eq!(preset("steam.exe", lib), None);
        assert_eq!(preset("steamwebhelper.exe", lib), None);
        assert_eq!(
            preset(
                "EpicGamesLauncher.exe",
                r"C:\Program Files (x86)\Epic Games\Launcher\Portal\Binaries\Win64\EpicGamesLauncher.exe"
            ),
            None
        );
        assert_eq!(
            preset(
                "EpicWebHelper.exe",
                r"C:\Program Files (x86)\Epic Games\Launcher\Engine\EpicWebHelper.exe"
            ),
            None
        );
        assert_eq!(
            preset(
                "Battle.net.exe",
                r"C:\Program Files (x86)\Battle.net\Battle.net.exe"
            ),
            None
        );
        assert_eq!(
            preset(
                "RiotClientServices.exe",
                r"C:\Riot Games\Riot Client\RiotClientServices.exe"
            ),
            None
        );
    }
}
