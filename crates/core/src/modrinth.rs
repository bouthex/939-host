//! Cliente de la API v2 de Modrinth. Solo metadatos reales: nada de compatibilidades inventadas.

use crate::error::{AppError, AppResult};
use crate::net;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;

pub const API: &str = "https://api.modrinth.com/v2";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct SearchHit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default = "unknown")]
    pub client_side: String,
    #[serde(default = "unknown")]
    pub server_side: String,
    #[serde(default)]
    pub categories: Vec<String>,
}

fn unknown() -> String { "unknown".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    pub offset: u32,
    pub limit: u32,
    pub total_hits: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct Project {
    pub id: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "unknown")]
    pub client_side: String,
    #[serde(default = "unknown")]
    pub server_side: String,
    #[serde(default)]
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct GalleryImage {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub featured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct License { #[serde(default)] pub id: String, #[serde(default)] pub name: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct ProjectDetail {
    pub id: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Descripción larga en Markdown (la interfaz la muestra como texto, sin HTML).
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default = "unknown")]
    pub client_side: String,
    #[serde(default = "unknown")]
    pub server_side: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub followers: u64,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub updated: String,
    #[serde(default)]
    pub license: Option<License>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub issues_url: Option<String>,
    #[serde(default)]
    pub wiki_url: Option<String>,
    #[serde(default)]
    pub gallery: Vec<GalleryImage>,
    #[serde(default)]
    pub team: String,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hashes {
    pub sha1: String,
    #[serde(default)]
    pub sha512: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionFile {
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub size: u64,
    pub hashes: Hashes,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DependencyType {
    Required,
    Optional,
    Incompatible,
    Embedded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct Dependency {
    #[serde(default)]
    pub version_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
    pub dependency_type: DependencyType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
    #[serde(default = "release")]
    pub version_type: String,
    #[serde(default)]
    pub date_published: String,
    #[serde(default)]
    pub files: Vec<VersionFile>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

fn release() -> String { "release".into() }

impl Version {
    pub fn primary_file(&self) -> Option<&VersionFile> {
        self.files.iter().find(|f| f.primary).or_else(|| self.files.first())
    }
    pub fn supports(&self, mc: &str, loader: &str) -> bool {
        self.game_versions.iter().any(|g| g == mc) && self.loaders.iter().any(|l| l == loader)
    }
}

/// Fuente de metadatos. La real habla con Modrinth; las pruebas usan datos fijos.
pub trait Source: Sync {
    fn version(&self, id: &str) -> impl Future<Output = AppResult<Version>> + Send;
    fn project_versions(&self, project: &str, mc: &str, loader: &str) -> impl Future<Output = AppResult<Vec<Version>>> + Send;
    fn projects(&self, ids: &[String]) -> impl Future<Output = AppResult<Vec<Project>>> + Send;
    fn versions_by_sha1(&self, hashes: &[String]) -> impl Future<Output = AppResult<HashMap<String, Version>>> + Send;
    /// Mods de la categoría «optimización» que corren en el servidor para esa versión exacta y
    /// loader, los más descargados primero (ver `optimize`). Sin red: ninguno.
    fn search_optimization(&self, _mc: &str, _loader: &str) -> impl Future<Output = AppResult<Vec<SearchHit>>> + Send { async { Ok(vec![]) } }
}

/// Cliente real de Modrinth.
pub struct Modrinth;

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl Modrinth {
    pub async fn search(&self, query: &str, mc: &str, loader: &str, server_only: bool, offset: u32) -> AppResult<SearchResult> {
        let mut facets = vec![
            vec!["project_type:mod".to_string()],
            vec![format!("categories:{loader}")],
            vec![format!("versions:{mc}")],
        ];
        if server_only { facets.push(vec!["server_side:required".into(), "server_side:optional".into()]); }
        let facets = serde_json::to_string(&facets)?;
        let url = format!("{API}/search?query={}&facets={}&limit=20&offset={offset}&index=relevance", enc(query), enc(&facets));
        net::get_json(&url, "Modrinth").await
    }

    /// Búsqueda para la tienda: categorías opcionales y filtro de lado ("server" | "client" | "both" | "").
    pub async fn search_filtered(&self, query: &str, mc: &str, loader: &str, categories: &[String], side: &str, sort: &str, offset: u32) -> AppResult<SearchResult> {
        let mut facets = vec![
            vec!["project_type:mod".to_string()],
            vec![format!("categories:{loader}")],
            vec![format!("versions:{mc}")],
        ];
        for c in categories.iter().filter(|c| c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') && !c.is_empty()).take(6) {
            facets.push(vec![format!("categories:{c}")]);
        }
        match side {
            "server" => { facets.push(vec!["server_side:required".into()]); facets.push(vec!["client_side:unsupported".into(), "client_side:optional".into()]); }
            "client" => { facets.push(vec!["client_side:required".into()]); facets.push(vec!["server_side:unsupported".into(), "server_side:optional".into()]); }
            "both" => { facets.push(vec!["server_side:required".into()]); facets.push(vec!["client_side:required".into()]); }
            _ => facets.push(vec!["server_side:required".into(), "server_side:optional".into()]),
        }
        let index = match sort { "downloads" | "follows" | "newest" | "updated" => sort, _ => "relevance" };
        let facets = serde_json::to_string(&facets)?;
        let url = format!("{API}/search?query={}&facets={}&limit=20&offset={offset}&index={index}", enc(query), enc(&facets));
        net::get_json(&url, "Modrinth").await
    }

    /// Modpacks de Modrinth que se pueden alojar (el servidor está soportado u opcional).
    pub async fn search_modpacks(&self, query: &str, offset: u32) -> AppResult<SearchResult> {
        let facets = serde_json::to_string(&vec![vec!["project_type:modpack".to_string()], vec!["server_side:required".into(), "server_side:optional".into()]])?;
        let url = format!("{API}/search?query={}&facets={}&limit=20&offset={offset}&index=relevance", enc(query), enc(&facets));
        net::get_json(&url, "Modrinth").await
    }
    /// Versiones de un modpack (la más nueva primero).
    pub async fn modpack_versions(&self, project: &str) -> AppResult<Vec<Version>> {
        net::get_json(&format!("{API}/project/{}/version", enc(project)), "Modrinth").await
    }

    /// Nombres de quienes forman el equipo de un proyecto (la persona dueña primero).
    pub async fn team_names(&self, team: &str) -> AppResult<Vec<String>> {
        if team.is_empty() || !team.chars().all(|c| c.is_ascii_alphanumeric()) { return Ok(vec![]); }
        let v: Vec<serde_json::Value> = net::get_json(&format!("{API}/team/{}/members", enc(team)), "Modrinth").await?;
        let mut out: Vec<(bool, String)> = v.iter().filter_map(|m| {
            let name = m["user"]["username"].as_str()?.to_string();
            Some((m["role"].as_str() != Some("Owner"), name))
        }).collect();
        out.sort();
        Ok(out.into_iter().map(|(_, n)| n).take(8).collect())
    }

    /// Ficha completa de un proyecto para la página de detalle.
    pub async fn project_detail(&self, id_or_slug: &str) -> AppResult<ProjectDetail> {
        net::get_json(&format!("{API}/project/{}", enc(id_or_slug)), "Modrinth").await
    }

    pub async fn project(&self, id_or_slug: &str) -> AppResult<Project> {
        net::get_json(&format!("{API}/project/{}", enc(id_or_slug)), "Modrinth").await
    }
}

impl Source for Modrinth {
    async fn search_optimization(&self, mc: &str, loader: &str) -> AppResult<Vec<SearchHit>> {
        // La versión exacta y, en Fabric/Quilt, también el resto de su serie (después se comprueba
        // en cada mod que acepte la versión exacta).
        let series = if matches!(loader, "fabric" | "quilt") { crate::mods::mc_series(mc) } else { vec![mc.to_string()] };
        let facets = serde_json::to_string(&vec![
            vec!["project_type:mod".to_string()], vec![format!("categories:{loader}")], vec!["categories:optimization".to_string()],
            series.iter().map(|v| format!("versions:{v}")).collect::<Vec<_>>(),
            vec!["server_side:required".into(), "server_side:optional".into()],
        ])?;
        let url = format!("{API}/search?query=&facets={}&limit=30&offset=0&index=downloads", enc(&facets));
        let r: SearchResult = net::get_json(&url, "Modrinth").await?;
        Ok(r.hits)
    }
    async fn version(&self, id: &str) -> AppResult<Version> {
        net::get_json(&format!("{API}/version/{}", enc(id)), "Modrinth").await
    }
    async fn project_versions(&self, project: &str, mc: &str, loader: &str) -> AppResult<Vec<Version>> {
        let url = format!("{API}/project/{}/version?loaders={}&game_versions={}", enc(project), enc(&format!("[\"{loader}\"]")), enc(&format!("[\"{mc}\"]")));
        net::get_json(&url, "Modrinth").await
    }
    async fn projects(&self, ids: &[String]) -> AppResult<Vec<Project>> {
        if ids.is_empty() { return Ok(vec![]); }
        let url = format!("{API}/projects?ids={}", enc(&serde_json::to_string(ids)?));
        net::get_json(&url, "Modrinth").await
    }
    async fn versions_by_sha1(&self, hashes: &[String]) -> AppResult<HashMap<String, Version>> {
        if hashes.is_empty() { return Ok(HashMap::new()); }
        let body = serde_json::json!({ "hashes": hashes, "algorithm": "sha1" });
        let resp = net::HTTP.post(format!("{API}/version_files")).json(&body).send().await
            .map_err(|e| AppError::network("No se pudo consultar Modrinth", e))?;
        if !resp.status().is_success() { return Err(AppError::network("Modrinth respondió con un error", resp.status())); }
        resp.json().await.map_err(|e| AppError::network("Respuesta inesperada de Modrinth", e))
    }
}

/// Elige la mejor versión compatible: la más reciente estable; si no hay, beta; si no, alfa.
pub fn best_version(mut versions: Vec<Version>, mc: &str, loader: &str) -> Option<Version> {
    versions.retain(|v| v.supports(mc, loader) && v.primary_file().is_some());
    versions.sort_by(|a, b| b.date_published.cmp(&a.date_published));
    for kind in ["release", "beta", "alpha"] {
        if let Some(v) = versions.iter().find(|v| v.version_type == kind) { return Some(v.clone()); }
    }
    None
}

/// ¿Puede ir en un servidor dedicado? Modrinth marca "unsupported" a los mods solo de cliente.
pub fn runs_on_server(server_side: &str) -> bool { server_side != "unsupported" }

/// Lado legible para la interfaz.
pub fn side_label(client: &str, server: &str) -> &'static str {
    match (client, server) {
        (_, "unsupported") => "client",
        ("unsupported", _) => "server",
        _ => "both",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Respuestas con el formato documentado de la API v2 (campos en snake_case).
    #[test]
    fn parses_documented_api_shapes() {
        let search = r#"{"hits":[{"project_id":"gvQqBUqZ","project_type":"mod","slug":"lithium","author":"jellysquid3","title":"Lithium","description":"No-compromises game logic optimization mod","categories":["fabric","optimization"],"display_categories":["optimization"],"versions":["26.2"],"downloads":102190000,"follows":22100,"icon_url":"https://cdn.modrinth.com/data/gvQqBUqZ/icon.png","date_created":"2021-01-03T00:53:34Z","date_modified":"2026-10-01T00:00:00Z","latest_version":"26.2","license":"LGPL-3.0-only","client_side":"optional","server_side":"optional","gallery":[]}],"offset":0,"limit":20,"total_hits":1}"#;
        let r: SearchResult = serde_json::from_str(search).unwrap();
        assert_eq!(r.hits[0].slug, "lithium");
        assert_eq!(r.total_hits, 1);
        let out = serde_json::to_value(&r).unwrap();
        assert!(out["hits"][0]["projectId"].is_string() && out["totalHits"] == 1, "la interfaz recibe camelCase");

        let version = r#"{"id":"IZskON6d","project_id":"AANobbMI","author_id":"x","featured":false,"name":"Sodium 0.9.2","version_number":"0.9.2+mc26.2","changelog":"","date_published":"2026-10-01T00:00:00Z","downloads":10,"version_type":"release","status":"listed","files":[{"hashes":{"sha512":"aa","sha1":"c84dd4b3580c02b79958a0590afd5783d80ef504"},"url":"https://cdn.modrinth.com/data/AANobbMI/versions/IZskON6d/sodium.jar","filename":"sodium-fabric-0.9.2+mc26.2.jar","primary":true,"size":1885572,"file_type":null}],"dependencies":[{"version_id":null,"project_id":"P7dR8mSH","file_name":null,"dependency_type":"required"},{"version_id":"abc","project_id":null,"file_name":null,"dependency_type":"embedded"}],"game_versions":["26.2"],"loaders":["fabric","quilt"]}"#;
        let v: Version = serde_json::from_str(version).unwrap();
        assert!(v.supports("26.2", "fabric") && !v.supports("26.2", "forge") && !v.supports("26.1", "fabric"));
        assert_eq!(v.primary_file().unwrap().hashes.sha1, "c84dd4b3580c02b79958a0590afd5783d80ef504");
        assert_eq!(v.dependencies[0].dependency_type, DependencyType::Required);
        assert_eq!(v.dependencies[1].dependency_type, DependencyType::Embedded);

        let project = r#"{"id":"AANobbMI","slug":"sodium","title":"Sodium","description":"x","client_side":"required","server_side":"unsupported","icon_url":null,"project_type":"mod"}"#;
        let p: Project = serde_json::from_str(project).unwrap();
        assert!(!runs_on_server(&p.server_side));
        assert_eq!(side_label(&p.client_side, &p.server_side), "client");
    }

    #[test]
    fn best_version_prefers_release_then_newest() {
        let mk = |id: &str, kind: &str, date: &str, mc: &str| Version { id: id.into(), project_id: "p".into(), name: id.into(), version_number: id.into(), game_versions: vec![mc.into()], loaders: vec!["fabric".into()], version_type: kind.into(), date_published: date.into(), files: vec![VersionFile { url: "u".into(), filename: "f.jar".into(), primary: true, size: 1, hashes: Hashes { sha1: "s".into(), sha512: String::new() } }], dependencies: vec![] };
        let list = vec![mk("beta-new", "beta", "2026-11-01", "26.2"), mk("rel-old", "release", "2026-01-01", "26.2"), mk("rel-new", "release", "2026-06-01", "26.2"), mk("other-mc", "release", "2026-12-01", "26.1")];
        assert_eq!(best_version(list.clone(), "26.2", "fabric").unwrap().id, "rel-new");
        assert_eq!(best_version(vec![list[0].clone()], "26.2", "fabric").unwrap().id, "beta-new");
        assert!(best_version(list, "26.3", "fabric").is_none());
    }
}
