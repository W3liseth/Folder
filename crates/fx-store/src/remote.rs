//! Messages de la fenêtre Paramètres (processus `folder.exe --settings`) vers la fenêtre
//! principale, qui reste la seule à écrire les réglages et la base des étiquettes.
//! Format texte simple : une lettre de type puis la charge utile.

use crate::{Settings, Tag, TagId};

#[derive(Debug, Clone, PartialEq)]
pub enum Remote {
    /// Nouveaux réglages complets.
    Settings(Box<Settings>),
    /// Étiquette créée (`id` 0) ou modifiée.
    SaveTag(Tag),
    DeleteTag(TagId),
}

impl Remote {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Settings(s) => format!("S{}", toml::to_string(s).unwrap_or_default()),
            Self::SaveTag(t) => format!(
                "T{}\t{},{},{}\t{}",
                t.id, t.color[0], t.color[1], t.color[2], t.name
            ),
            Self::DeleteTag(id) => format!("D{id}"),
        }
        .into_bytes()
    }

    /// Décode un message ; `None` s'il est invalide (il vient d'un autre processus).
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let text = std::str::from_utf8(bytes).ok()?;
        let (kind, rest) = text.split_at_checked(1)?;
        match kind {
            "S" => toml::from_str(rest)
                .ok()
                .map(|s| Self::Settings(Box::new(s))),
            "T" => {
                let mut parts = rest.splitn(3, '\t');
                let id = parts.next()?.parse().ok()?;
                let mut rgb = parts.next()?.split(',').map(|c| c.parse::<u8>());
                let color = [rgb.next()?.ok()?, rgb.next()?.ok()?, rgb.next()?.ok()?];
                let name = parts.next()?.trim().to_owned();
                (!name.is_empty()).then_some(Self::SaveTag(Tag { id, name, color }))
            }
            "D" => rest.parse().ok().map(Self::DeleteTag),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = Settings {
            show_hidden: true,
            accent: Some([1, 2, 3]),
            ..Settings::default()
        };
        for r in [
            Remote::Settings(Box::new(s)),
            Remote::SaveTag(Tag {
                id: 0,
                name: "Travail\tà faire".into(),
                color: [0, 120, 212],
            }),
            Remote::DeleteTag(7),
        ] {
            assert_eq!(Remote::decode(&r.encode()), Some(r));
        }
        assert_eq!(Remote::decode(b"Xgarbage"), None);
        assert_eq!(Remote::decode(b""), None);
        assert_eq!(Remote::decode(b"T1\t300,0,0\tnom"), None);
    }
}
