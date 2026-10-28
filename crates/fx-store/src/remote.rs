//! Messages entre processus folder : de la fenêtre Paramètres (`folder.exe --settings`) vers
//! la fenêtre principale, qui reste la seule à écrire les réglages et la base des
//! étiquettes ; et d'une fenêtre principale à une autre (onglet glissé de l'une à l'autre).
//! Format texte simple : une lettre de type puis la charge utile.

use std::path::PathBuf;

use crate::{Settings, Tag, TagId};

#[derive(Debug, Clone, PartialEq)]
pub enum Remote {
    /// Nouveaux réglages complets.
    Settings(Box<Settings>),
    /// Étiquette créée (`id` 0) ou modifiée.
    SaveTag(Tag),
    DeleteTag(TagId),
    /// Onglet d'une autre fenêtre survolant la barre d'onglets : pointeur en `at` (pixels
    /// écran), saisi en `grab` (points, depuis son coin haut gauche), emplacement `path`
    /// (cf. `Location::window_arg`). La barre le montre sous le pointeur, comme un des siens.
    TabHover {
        at: [i32; 2],
        grab: [i32; 2],
        path: PathBuf,
    },
    /// Fin du survol (pointeur parti, glisser abandonné).
    TabHoverEnd,
    /// Onglet lâché sur la barre d'onglets (mêmes `at`, `grab`, `path` que le survol), avec
    /// l'historique de son volet (cf. `History::saved`).
    TabDrop {
        at: [i32; 2],
        grab: [i32; 2],
        path: PathBuf,
        back: Vec<PathBuf>,
        forward: Vec<PathBuf>,
    },
    /// Installer la mise à jour téléchargée : la fenêtre principale lance l'installeur et se
    /// ferme en enregistrant sa session.
    InstallUpdate,
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
            Self::TabHover { at, grab, path } => {
                format!("H{}\n{}", numbers(at, grab), path.display())
            }
            Self::TabHoverEnd => "E".to_owned(),
            Self::InstallUpdate => "U".to_owned(),
            // Une ligne par chemin (un chemin Windows ne contient pas de saut de ligne) :
            // positions, emplacement, puis l'historique préfixé par `<` ou `>`.
            Self::TabDrop {
                at,
                grab,
                path,
                back,
                forward,
            } => {
                let mut s = format!("A{}\n{}", numbers(at, grab), path.display());
                for (mark, list) in [('<', back), ('>', forward)] {
                    for p in list {
                        s.push('\n');
                        s.push(mark);
                        s.push_str(&p.to_string_lossy());
                    }
                }
                s
            }
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
            "H" => {
                let (nums, path) = rest.split_once('\n')?;
                let (at, grab) = parse_numbers(nums)?;
                (!path.is_empty() && !path.contains('\n')).then(|| Self::TabHover {
                    at,
                    grab,
                    path: PathBuf::from(path),
                })
            }
            "E" => rest.is_empty().then_some(Self::TabHoverEnd),
            "U" => rest.is_empty().then_some(Self::InstallUpdate),
            "A" => {
                let mut lines = rest.split('\n');
                let (at, grab) = parse_numbers(lines.next()?)?;
                let path = PathBuf::from(lines.next().filter(|p| !p.is_empty())?);
                let (mut back, mut forward) = (Vec::new(), Vec::new());
                for l in lines {
                    match l.split_at_checked(1)? {
                        ("<", p) => back.push(PathBuf::from(p)),
                        (">", p) => forward.push(PathBuf::from(p)),
                        _ => return None,
                    }
                }
                Some(Self::TabDrop {
                    at,
                    grab,
                    path,
                    back,
                    forward,
                })
            }
            _ => None,
        }
    }
}

/// `x,y,gx,gy` : position du pointeur et point de saisie.
fn numbers(at: &[i32; 2], grab: &[i32; 2]) -> String {
    format!("{},{},{},{}", at[0], at[1], grab[0], grab[1])
}

/// Relit [`numbers`] (entiers signés : écrans à gauche ou au-dessus du principal).
fn parse_numbers(s: &str) -> Option<([i32; 2], [i32; 2])> {
    let n: Vec<i32> = s
        .split(',')
        .map(|v| v.parse().ok())
        .collect::<Option<_>>()?;
    match n[..] {
        [x, y, gx, gy] => Some(([x, y], [gx, gy])),
        _ => None,
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
            Remote::TabHover {
                at: [-1200, 30],
                grab: [52, 16],
                path: r"C:\Users\moi\Documents".into(),
            },
            Remote::TabHoverEnd,
            Remote::InstallUpdate,
            Remote::TabDrop {
                at: [640, 22],
                grab: [10, 3],
                path: r"C:\Users\moi\Été 2026".into(),
                back: vec!["folder:home".into(), r"C:\Users".into()],
                forward: vec![r"D:\".into()],
            },
            Remote::TabDrop {
                at: [0, 0],
                grab: [0, 0],
                path: "folder:home".into(),
                back: vec![],
                forward: vec![],
            },
        ] {
            assert_eq!(Remote::decode(&r.encode()), Some(r));
        }
        assert_eq!(Remote::decode(b"Xgarbage"), None);
        assert_eq!(Remote::decode(b""), None);
        assert_eq!(Remote::decode(b"T1\t300,0,0\tnom"), None);
        assert_eq!(Remote::decode(b"A1,2,3,4\n"), None);
        assert_eq!(Remote::decode(b"A1,2,3,4\nC:\\x\n?y"), None);
        assert_eq!(Remote::decode(b"H1,2\nC:\\x"), None);
        assert_eq!(Remote::decode(b"Hx,2,3,4\nC:\\x"), None);
        assert_eq!(Remote::decode(b"H1,2,3,4\n"), None);
    }
}
