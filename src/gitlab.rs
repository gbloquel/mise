use crate::cache::{CacheManager, CacheManagerBuilder};
use crate::{dirs, duration, env};
use eyre::Result;
use heck::ToKebabCase;
use reqwest::header::HeaderMap;
use serde_derive::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock as Lazy;
use std::sync::{RwLock, RwLockReadGuard};
use xx::regex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitlabRelease {
    pub tag_name: String,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<GitlabAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitlabTag {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitlabAsset {
    pub name: String,
    // pub size: u64,
    pub browser_download_url: String,
}

type CacheGroup<T> = HashMap<String, CacheManager<T>>;

static RELEASES_CACHE: Lazy<RwLock<CacheGroup<Vec<GitlabRelease>>>> = Lazy::new(Default::default);

static RELEASE_CACHE: Lazy<RwLock<CacheGroup<GitlabRelease>>> = Lazy::new(Default::default);

static TAGS_CACHE: Lazy<RwLock<CacheGroup<Vec<String>>>> = Lazy::new(Default::default);

fn get_tags_cache(key: &str) -> RwLockReadGuard<'_, CacheGroup<Vec<String>>> {
    TAGS_CACHE
        .write()
        .unwrap()
        .entry(key.to_string())
        .or_insert_with(|| {
            CacheManagerBuilder::new(cache_dir().join(format!("{key}-tags.msgpack.z")))
                .with_fresh_duration(Some(duration::DAILY))
                .build()
        });
    TAGS_CACHE.read().unwrap()
}

fn get_releases_cache(key: &str) -> RwLockReadGuard<'_, CacheGroup<Vec<GitlabRelease>>> {
    RELEASES_CACHE
        .write()
        .unwrap()
        .entry(key.to_string())
        .or_insert_with(|| {
            CacheManagerBuilder::new(cache_dir().join(format!("{key}-releases.msgpack.z")))
                .with_fresh_duration(Some(duration::DAILY))
                .build()
        });
    RELEASES_CACHE.read().unwrap()
}

fn get_release_cache<'a>(key: &str) -> RwLockReadGuard<'a, CacheGroup<GitlabRelease>> {
    RELEASE_CACHE
        .write()
        .unwrap()
        .entry(key.to_string())
        .or_insert_with(|| {
            CacheManagerBuilder::new(cache_dir().join(format!("{key}.msgpack.z")))
                .with_fresh_duration(Some(duration::DAILY))
                .build()
        });
    RELEASE_CACHE.read().unwrap()
}

pub fn list_releases(repo: &str) -> Result<Vec<GitlabRelease>> {
    let key = repo.to_kebab_case();
    let cache = get_releases_cache(&key);
    let cache = cache.get(&key).unwrap();
    Ok(cache.get_or_try_init(|| list_releases_(repo))?.to_vec())
}

fn list_releases_(repo: &str) -> Result<Vec<GitlabRelease>> {
    //TODO call api gitlab convert repo to GET /projects/:id/releases
    // see https://docs.gitlab.com/ee/api/releases/
    let url = format!("https://api.github.com/repos/{repo}/releases");
    let (mut releases, mut headers) =
        crate::http::HTTP_FETCH.json_headers::<Vec<GitlabRelease>, _>(url)?;

    if *env::MISE_LIST_ALL_VERSIONS {
        while let Some(next) = next_page(&headers) {
            let (more, h) = crate::http::HTTP_FETCH.json_headers::<Vec<GitlabRelease>, _>(next)?;
            releases.extend(more);
            headers = h;
        }
    }
    releases.retain(|r| !r.draft && !r.prerelease);

    Ok(releases)
}

pub fn list_tags(repo: &str) -> Result<Vec<String>> {
    let key = repo.to_kebab_case();
    let cache = get_tags_cache(&key);
    let cache = cache.get(&key).unwrap();
    Ok(cache.get_or_try_init(|| list_tags_(repo))?.to_vec())
}

fn list_tags_(repo: &str) -> Result<Vec<String>> {
    let url = format!("https://api.github.com/repos/{}/tags", repo);
    let (mut tags, mut headers) = crate::http::HTTP_FETCH.json_headers::<Vec<GitlabTag>, _>(url)?;

    if *env::MISE_LIST_ALL_VERSIONS {
        while let Some(next) = next_page(&headers) {
            let (more, h) = crate::http::HTTP_FETCH.json_headers::<Vec<GitlabTag>, _>(next)?;
            tags.extend(more);
            headers = h;
        }
    }

    Ok(tags.into_iter().map(|t| t.name).collect())
}

pub fn get_release(repo: &str, tag: &str, api_base_url: &str) -> Result<GitlabRelease> {
    let key = format!("{repo}-{tag}").to_kebab_case();
    let cache = get_release_cache(&key);
    let cache = cache.get(&key).unwrap();
    Ok(cache.get_or_try_init(|| get_release_(repo, tag, api_base_url))?.clone())
}

fn get_release_(repo: &str, tag: &str, api_base_url: &str) -> Result<GitlabRelease> {
    let url = format!("{api_base_url}/projects/{repo}/releases/{tag}");
    crate::http::HTTP_FETCH.json(url)
}

fn next_page(headers: &HeaderMap) -> Option<String> {
    let link = headers
        .get("link")
        .map(|l| l.to_str().unwrap_or_default().to_string())
        .unwrap_or_default();
    regex!(r#"<([^>]+)>; rel="next""#)
        .captures(&link)
        .map(|c| c.get(1).unwrap().as_str().to_string())
}

fn cache_dir() -> PathBuf {
    dirs::CACHE.join("gitlab")
}
