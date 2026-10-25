//! Private, per-request cookie context. Never stored as a login Session.
use framer_core::ids::FramerUserId;
use framer_core::ids::ProjectId;
use framer_core::port::Session;

pub struct CredentialTarget {
    pub project: ProjectId,
    pub path: &'static str,
}

impl CredentialTarget {
    pub(crate) fn valid_for_browser(&self) -> bool {
        matches!(self.path, "/ai/v3/responses/" | "/ai/v3/messages/")
            && self.valid_project()
    }

    fn valid_project(&self) -> bool {
        !self.project.0.is_empty() && self.project.0.len() <= 256
            && self.project.0.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    }

    pub(crate) fn matches_page(&self, page: &str) -> bool {
        self.valid_for_browser() && self.matches_project_page(page)
    }

    pub(crate) fn matches_offline_page(&self, page: &str) -> bool {
        self.valid_project() && framer_api::routes::inference().any(|route| route.path == self.path)
            && self.matches_project_page(page)
    }

    fn matches_project_page(&self, page: &str) -> bool {
        if page.len() > 4096 { return false; }
        let Ok(url) = reqwest::Url::parse(page) else { return false; };
        if url.scheme() != "https" || url.host_str() != Some("framer.com")
            || !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
            return false;
        }
        let path = url.path().trim_end_matches('/');
        let Some(project) = path.strip_prefix("/projects/") else { return false; };
        !project.contains('/') && (project == self.project.0
            || project.ends_with(&format!("--{}", self.project.0))
            || matches_project_access_link(project, &self.project.0))
    }
}

fn matches_project_access_link(segment: &str, expected: &str) -> bool {
    let alphanumeric = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric());
    if expected.len() != 20 || !alphanumeric(expected) { return false; }
    let Some((with_id, access)) = segment.rsplit_once('-') else { return false; };
    if !alphanumeric(access) { return false; }
    let (slug, id) = match with_id.rsplit_once("--") {
        Some((slug, id)) => (Some(slug), id),
        None => (None, with_id),
    };
    id == expected && slug.map_or(true, |s| s.split('-').all(alphanumeric))
}

/// A bounded jar snapshot tied to the exact private session that seeded it.
/// Construction rejects a changed or ambiguous login cookie.
pub struct CurrentRequestCookies {
    seeded_user: FramerUserId,
    seeded_header: String,
    current_header: String,
}

impl CurrentRequestCookies {
    pub(crate) fn from_saved_device(session: &Session, device: &str) -> Result<Self, String> {
        if device.is_empty() || device.len() > 256 || !device.bytes().all(|b|
            b.is_ascii_alphanumeric() || b"._~-".contains(&b)) || !Self::valid_session(session) {
            return Err("invalid saved request device".into());
        }
        let mut present = false;
        for part in session.cookie_header.split(';') {
            let (name, value) = part.trim().split_once('=')
                .ok_or("invalid saved request cookies")?;
            if name == "__cuid" {
                if present || value != device { return Err("conflicting request device".into()); }
                present = true;
            }
        }
        let header = if present { session.cookie_header.clone() }
            else { format!("{}; __cuid={device}", session.cookie_header) };
        Self::from_current_jar(session, header)
    }

    pub(crate) fn valid_session(session: &Session) -> bool {
        login_cookie(&session.cookie_header).is_some()
    }

    pub fn from_current_jar(session: &Session, current_header: String) -> Result<Self, String> {
        let original = login_cookie(&session.cookie_header)
            .ok_or("invalid original login cookie")?;
        let current = login_cookie(&current_header).ok_or("invalid current login cookie")?;
        if original != current { return Err("current jar changed login identity".into()); }
        Ok(Self { seeded_user: session.active_user.clone(),
            seeded_header: session.cookie_header.clone(), current_header })
    }

    pub(crate) fn into_session(self, expected: &Session) -> Result<Session, String> {
        if self.seeded_user != expected.active_user || self.seeded_header != expected.cookie_header {
            return Err("request cookies belong to another private session".into());
        }
        Ok(Session { active_user: expected.active_user.clone(), cookie_header: self.current_header })
    }
}

fn login_cookie(header: &str) -> Option<&str> {
    if header.is_empty() || header.len() > 16 * 1024
        || header.bytes().any(|b| !(0x20..=0x7e).contains(&b)) { return None; }
    let mut session = None;
    for part in header.split(';') {
        let (name, value) = part.trim().split_once('=')?;
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric()
            || b"!#$%&'*+-.^_`|~".contains(&b))
            || value.bytes().any(|b| b < 0x21 || b == b',' || b == b'\\' || b == b'"') {
            return None;
        }
        if name == "session" {
            if value.is_empty() || session.replace(value).is_some() { return None; }
        }
    }
    session
}

#[cfg(test)]
#[path = "credential_target_tests.rs"]
mod tests;
