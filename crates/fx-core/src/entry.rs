use crate::names::{NameArena, NameSpan};

/// Valeurs `FILE_ATTRIBUTE_*` utiles (dupliquées ici pour garder `fx-core` sans dépendance Windows).
pub mod attrs {
    pub const READONLY: u32 = 0x1;
    pub const HIDDEN: u32 = 0x2;
    pub const SYSTEM: u32 = 0x4;
    pub const DIRECTORY: u32 = 0x10;
    pub const REPARSE_POINT: u32 = 0x400;
    pub const OFFLINE: u32 = 0x1000;
    pub const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
}

/// Indicateurs dérivés des attributs, testés à chaque image : gardés sur 16 bits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EntryFlags(u16);

impl EntryFlags {
    pub const DIR: Self = Self(1);
    pub const HIDDEN: Self = Self(1 << 1);
    pub const SYSTEM: Self = Self(1 << 2);
    pub const LINK: Self = Self(1 << 3);
    /// Fichier cloud non hydraté (OneDrive « à la demande »…).
    pub const CLOUD: Self = Self(1 << 4);

    pub fn from_attributes(a: u32) -> Self {
        let mut f = 0;
        if a & attrs::DIRECTORY != 0 {
            f |= Self::DIR.0;
        }
        if a & attrs::HIDDEN != 0 {
            f |= Self::HIDDEN.0;
        }
        if a & attrs::SYSTEM != 0 {
            f |= Self::SYSTEM.0;
        }
        if a & attrs::REPARSE_POINT != 0 {
            f |= Self::LINK.0;
        }
        if a & (attrs::OFFLINE | attrs::RECALL_ON_DATA_ACCESS) != 0 {
            f |= Self::CLOUD.0;
        }
        Self(f)
    }

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Extension internée par instantané (`0` = aucune). Sert aussi de clé au cache d'icônes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ExtId(pub u16);

impl ExtId {
    pub const NONE: Self = Self(0);
}

/// Métadonnées d'une entrée : **40 octets**, sans allocation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct EntryMeta {
    pub name: NameSpan,
    pub size: u64,
    /// FILETIME UTC brut.
    pub modified: i64,
    pub created: i64,
    pub attributes: u32,
    pub ext: ExtId,
    pub flags: EntryFlags,
}

const _: () = assert!(
    size_of::<EntryMeta>() == 40,
    "budget : 40 octets par entrée"
);

impl EntryMeta {
    pub fn is_dir(&self) -> bool {
        self.flags.contains(EntryFlags::DIR)
    }
}

/// Lot d'entrées produit par un worker d'énumération et fusionné par l'UI.
#[derive(Debug, Default)]
pub struct Batch {
    pub names: NameArena,
    pub entries: Vec<EntryMeta>,
    /// Dossier des entrées, pour les listes virtuelles (résultats de recherche) ;
    /// `None` : le dossier de l'instantané.
    pub dir: Option<std::path::PathBuf>,
}

impl Batch {
    pub fn with_capacity(entries: usize) -> Self {
        Self {
            names: NameArena::with_capacity(entries * 24),
            entries: Vec::with_capacity(entries),
            dir: None,
        }
    }

    pub fn push_utf16(
        &mut self,
        name: &[u16],
        attributes: u32,
        size: u64,
        created: i64,
        modified: i64,
    ) {
        let name = self.names.push_utf16(name);
        self.entries.push(EntryMeta {
            name,
            size,
            modified,
            created,
            attributes,
            ext: ExtId::NONE,
            flags: EntryFlags::from_attributes(attributes),
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
