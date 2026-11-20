use crate::engine::EngineClient;
use crate::error::{Error, Result};
use crate::gamesense::GameSenseClient;
use crate::http::{Http, pem_to_der};
use crate::sonar::SonarClient;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(1500);

/// `%PROGRAMDATA%\SteelSeries\GG\coreProps.json`, rewritten by GG on every start.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CoreProps {
    /// GameSense, plain HTTP.
    pub address: Option<String>,
    /// Engine, HTTPS.
    #[serde(rename = "encryptedAddress")]
    pub encrypted_address: Option<String>,
    /// GG itself, HTTPS.
    #[serde(rename = "ggEncryptedAddress")]
    pub gg_encrypted_address: Option<String>,
}

pub fn core_props_path() -> PathBuf {
    let base = std::env::var_os("PROGRAMDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    base.join("SteelSeries").join("GG").join("coreProps.json")
}

pub fn read_core_props() -> Result<CoreProps> {
    let path = core_props_path();
    let text = std::fs::read_to_string(&path).map_err(|e| Error::Offline(format!("{}: {e}", path.display())))?;
    serde_json::from_str(&text).map_err(|e| Error::Parse(e.to_string()))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAppMetadata {
    #[serde(default)]
    pub encrypted_web_server_address: String,
    #[serde(default)]
    pub web_server_address: String,
    #[serde(default)]
    pub offline_frontend_address: String,
    #[serde(default)]
    pub online_frontend_address: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAppSecrets {
    #[serde(default)]
    pub encrypted_web_server_address_cert_text: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubApp {
    pub name: String,
    #[serde(default)]
    pub is_enabled: bool,
    #[serde(default)]
    pub is_ready: bool,
    #[serde(default)]
    pub is_running: bool,
    pub metadata: Option<SubAppMetadata>,
    pub secret_metadata: Option<SubAppSecrets>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubApps {
    sub_apps: BTreeMap<String, SubApp>,
}

/// Where every SteelSeries service is listening right now, with a client for each.
#[derive(Clone, Default)]
pub struct Endpoints {
    pub gg: Option<String>,
    pub sonar: Option<String>,
    pub engine: Option<String>,
    pub engine_cert_pem: Option<String>,
    pub gamesense: Option<String>,
    pub sub_apps: BTreeMap<String, SubApp>,
    sonar_client: Option<SonarClient>,
    engine_client: Option<EngineClient>,
    gamesense_client: Option<GameSenseClient>,
}

impl Endpoints {
    /// Reads coreProps.json, then asks GG (`GET /subApps`) for Sonar's address and Engine's certificate.
    pub fn discover() -> Result<Self> {
        let props = read_core_props()?;
        let mut ep = Endpoints {
            gg: props.gg_encrypted_address.as_ref().map(|a| format!("https://{a}")),
            engine: props.encrypted_address.as_ref().map(|a| format!("https://{a}")),
            gamesense: props.address.as_ref().map(|a| format!("http://{a}")),
            ..Default::default()
        };
        if let Some(gg) = &ep.gg {
            let http = Http::new(None, DEFAULT_TIMEOUT);
            let apps: SubApps = http.get_json(&format!("{gg}/subApps"))?;
            if let Some(s) = apps.sub_apps.get("sonar").and_then(|s| s.metadata.as_ref())
                && !s.web_server_address.is_empty()
            {
                ep.sonar = Some(s.web_server_address.trim_end_matches('/').to_string());
            }
            if let Some(e) = apps.sub_apps.get("engine") {
                if let Some(m) = &e.metadata
                    && !m.encrypted_web_server_address.is_empty()
                {
                    ep.engine = Some(format!("https://{}", m.encrypted_web_server_address));
                }
                ep.engine_cert_pem = e
                    .secret_metadata
                    .as_ref()
                    .map(|s| s.encrypted_web_server_address_cert_text.clone())
                    .filter(|c| !c.is_empty());
            }
            ep.sub_apps = apps.sub_apps;
        }
        ep.sonar_client = ep.sonar.as_deref().map(SonarClient::new);
        let der = ep.engine_cert_pem.as_deref().and_then(pem_to_der);
        ep.engine_client = ep.engine.as_deref().map(|b| EngineClient::new(b, der));
        ep.gamesense_client = ep.gamesense.as_deref().map(GameSenseClient::new);
        Ok(ep)
    }

    /// Clones share the connection pool and write pacing; keep the `Endpoints` around.
    pub fn sonar(&self) -> Option<SonarClient> {
        self.sonar_client.clone()
    }

    pub fn engine(&self) -> Option<EngineClient> {
        self.engine_client.clone()
    }

    pub fn gamesense(&self) -> Option<GameSenseClient> {
        self.gamesense_client.clone()
    }
}
