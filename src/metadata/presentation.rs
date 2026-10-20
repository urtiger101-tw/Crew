//! Source-scoped display patches. This module never changes an agent's lifecycle state.
use super::*;
use serde::{Deserializer, Serializer};

/// Missing, explicit null, and a string have distinct patch semantics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextPatch(pub Option<Option<String>>);

impl TextPatch {
    pub fn missing(&self) -> bool {
        self.0.is_none()
    }
    pub fn set(value: String) -> Self {
        Self(Some(Some(value)))
    }
    pub fn clear() -> Self {
        Self(Some(None))
    }
}
impl<'de> Deserialize<'de> for TextPatch {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Option::<String>::deserialize(d).map(|value| Self(Some(value)))
    }
}
impl Serialize for TextPatch {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.as_ref().unwrap_or(&None).serialize(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Presentation {
    pub title: Option<String>,
    pub display_agent: Option<String>,
    pub state_labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub value: Presentation,
    pub agent: Option<String>,
    pub authority_source: Option<String>,
    pub deadline: Option<Instant>,
    pub revision: u64,
}

pub fn clean(value: &str, max: usize) -> Option<String> {
    let text: String = value
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(max)
        .collect();
    clean_text(&text, max)
}

pub(super) fn source_valid(source: &str) -> bool {
    !source.is_empty()
        && source.len() <= 80
        && source
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'.' | b'_' | b'-'))
}

impl Request {
    pub(super) fn patches_presentation(&self) -> bool {
        !self.title.missing()
            || !self.display_agent.missing()
            || !self.state_labels.is_empty()
            || self.agent.is_some()
            || self.applies_to_source.is_some()
            || self.clear_presentation
    }

    pub(super) fn validate_presentation(&self) -> Result<(), ApiError> {
        if self.state_labels.keys().any(|key| {
            !matches!(
                key.as_str(),
                "idle" | "working" | "blocked" | "done" | "unknown"
            )
        }) {
            return Err(ApiError::invalid(
                "state labels must use idle|working|blocked|done|unknown",
            ));
        }
        if self
            .agent
            .as_ref()
            .is_some_and(|s| clean(s, 80).as_deref() != Some(s.as_str()))
        {
            return Err(ApiError::invalid(
                "agent guard must be clean, nonempty text of at most 80 characters",
            ));
        }
        if self
            .applies_to_source
            .as_ref()
            .is_some_and(|s| !source_valid(s))
        {
            return Err(ApiError::invalid(
                "applies-to-source must be a source identifier",
            ));
        }
        if self.clear_presentation
            && (!self.title.missing()
                || !self.display_agent.missing()
                || !self.state_labels.is_empty()
                || self.agent.is_some()
                || self.applies_to_source.is_some())
        {
            return Err(ApiError::invalid(
                "clear-presentation cannot be combined with presentation setters or guards",
            ));
        }
        if matches!(self.target, Target::Workspace(_))
            && (!self.display_agent.missing()
                || !self.state_labels.is_empty()
                || self.agent.is_some()
                || self.applies_to_source.is_some())
        {
            return Err(ApiError::invalid(
                "workspace presentation supports title only",
            ));
        }
        Ok(())
    }
}

impl Store {
    pub(crate) fn presentation(
        &self,
        key: Key,
        now: Instant,
        agent: Option<&str>,
        source: Option<&str>,
    ) -> Presentation {
        let Some(resource) = self.resources.get(&key) else {
            return Presentation::default();
        };
        let mut entries: Vec<_> = resource
            .presentations
            .values()
            .filter(|entry| {
                entry.deadline.is_none_or(|deadline| now < deadline)
                    && entry
                        .agent
                        .as_deref()
                        .is_none_or(|guard| Some(guard) == agent)
                    && entry
                        .authority_source
                        .as_deref()
                        .is_none_or(|guard| Some(guard) == source)
            })
            .collect();
        entries.sort_by_key(|entry| entry.revision);
        let mut result = Presentation::default();
        for entry in entries {
            if entry.value.title.is_some() {
                result.title.clone_from(&entry.value.title);
            }
            if entry.value.display_agent.is_some() {
                result.display_agent.clone_from(&entry.value.display_agent);
            }
            result.state_labels.extend(entry.value.state_labels.clone());
        }
        result
    }
}

pub(super) fn patch(
    resource: &mut Resource,
    req: &Request,
    now: Instant,
    deadline: Option<Instant>,
    revision: u64,
) -> bool {
    if !req.patches_presentation() {
        return false;
    }
    if req.clear_presentation {
        return resource.presentations.remove(&req.source).is_some();
    }
    let before = resource.presentations.get(&req.source).cloned();
    // Updating after expiry starts a fresh source presentation even when the timer
    // has not swept yet. Otherwise omitted labels/guards could revive by timing.
    let mut next = before
        .clone()
        .filter(|old| old.deadline.is_none_or(|deadline| now < deadline))
        .unwrap_or(Entry {
            value: Presentation::default(),
            agent: None,
            authority_source: None,
            deadline: None,
            revision,
        });
    if let Some(value) = &req.title.0 {
        next.value.title = value.as_deref().and_then(|v| clean(v, 80));
    }
    if let Some(value) = &req.display_agent.0 {
        next.value.display_agent = value.as_deref().and_then(|v| clean(v, 80));
    }
    for (key, value) in &req.state_labels {
        if let Some(value) = value.as_deref().and_then(|v| clean(v, 80)) {
            next.value.state_labels.insert(key.clone(), value);
        } else {
            next.value.state_labels.remove(key);
        }
    }
    if req.agent.is_some() {
        next.agent.clone_from(&req.agent);
    }
    if req.applies_to_source.is_some() {
        next.authority_source.clone_from(&req.applies_to_source);
    }
    next.deadline = deadline;
    // An identical refresh without a TTL does not reorder publishers or emit an event.
    if before.as_ref().is_some_and(|old| {
        old.value == next.value
            && old.agent == next.agent
            && old.authority_source == next.authority_source
            && old.deadline == next.deadline
    }) {
        return false;
    }
    next.revision = revision;
    // Guards are independent patch state. Clearing the last visible field must
    // not silently make a later patch apply to another agent or report source.
    if next.value == Presentation::default()
        && next.agent.is_none()
        && next.authority_source.is_none()
    {
        resource.presentations.remove(&req.source);
    } else {
        resource.presentations.insert(req.source.clone(), next);
    }
    before != resource.presentations.get(&req.source).cloned()
}
