//! Dépôts Git : branche courante (lecture de `.git/HEAD`, sans programme externe), liste des
//! branches et changement de branche.
//!
//! La liste et le changement passent par le programme `git` de l'utilisateur (Git pour
//! Windows) : il gère seul les références compactées, les worktrees et la mise à jour de
//! l'arbre de travail. Aucune commande qui touche au réseau (`fetch`, `pull`) n'est lancée.
//! Appels bloquants : à exécuter hors du thread UI.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Branches d'un dépôt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitBranches {
    /// Branche active (`None` : HEAD détachée).
    pub current: Option<String>,
    /// Branches locales, de la plus récemment modifiée à la plus ancienne.
    pub local: Vec<String>,
    /// Branches distantes sans branche locale du même nom (`origin/x`) : les choisir crée la
    /// branche locale qui la suit (`git switch x`).
    pub remote: Vec<String>,
}

impl GitBranches {
    /// Nom à passer à `git switch` pour une branche distante `origin/x`.
    pub fn remote_target(remote: &str) -> &str {
        remote.split_once('/').map_or(remote, |(_, b)| b)
    }
}

/// Branche Git du dépôt contenant `dir` (lecture de `.git/HEAD`) ; HEAD détachée : les sept
/// premiers caractères du commit.
pub fn branch(dir: &Path) -> Option<String> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        let git = d.join(".git");
        let head = if git.is_dir() {
            std::fs::read_to_string(git.join("HEAD")).ok()
        } else if git.is_file() {
            // Sous-module ou worktree : « gitdir: <chemin> ».
            let link = std::fs::read_to_string(&git).ok()?;
            let target = link.strip_prefix("gitdir:")?.trim();
            let target = if Path::new(target).is_absolute() {
                PathBuf::from(target)
            } else {
                d.join(target)
            };
            std::fs::read_to_string(target.join("HEAD")).ok()
        } else {
            None
        };
        if let Some(h) = head {
            let h = h.trim();
            return Some(match h.strip_prefix("ref: refs/heads/") {
                Some(b) => b.to_owned(),
                None => h.chars().take(7).collect(),
            });
        }
        cur = d.parent();
    }
    None
}

/// Branches locales et distantes du dépôt contenant `dir`.
pub fn branches(dir: &Path) -> Result<GitBranches, String> {
    let out = run(
        dir,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(HEAD)%09%(refname)",
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    Ok(parse_refs(&out))
}

/// Active `branch` (`git switch`). Les modifications locales qui entreraient en conflit
/// font échouer la commande (message de Git renvoyé) : rien n'est perdu.
pub fn switch(dir: &Path, branch: &str) -> Result<(), String> {
    run(dir, &["switch", branch]).map(|_| ())
}

fn parse_refs(out: &str) -> GitBranches {
    let mut b = GitBranches::default();
    let mut remotes = Vec::new();
    for line in out.lines() {
        let Some((head, name)) = line.split_once('\t') else {
            continue;
        };
        if let Some(local) = name.strip_prefix("refs/heads/") {
            if head == "*" {
                b.current = Some(local.to_owned());
            }
            b.local.push(local.to_owned());
        } else if let Some(remote) = name.strip_prefix("refs/remotes/")
            && !remote.ends_with("/HEAD")
            && remote.contains('/')
        {
            remotes.push(remote.to_owned());
        }
    }
    for r in remotes {
        let target = GitBranches::remote_target(&r);
        let known = b.local.iter().any(|l| l == target)
            || b.remote
                .iter()
                .any(|x| GitBranches::remote_target(x) == target);
        if !known {
            b.remote.push(r);
        }
    }
    b
}

fn run(dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(args)
        // Jamais d'invite (identifiants, éditeur) : il n'y a pas de console pour y répondre.
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW : pas de console qui apparaît le temps de la commande.
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "Git est introuvable (installez Git pour Windows)".to_owned()
        } else {
            e.to_string()
        }
    })?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(git_error(&err))
}

/// Première ligne utile d'un message d'erreur de Git, sans son préfixe.
fn git_error(stderr: &str) -> String {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("error:") || l.starts_with("fatal:"))
        .or_else(|| stderr.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("échec de Git");
    line.trim_start_matches("error:")
        .trim_start_matches("fatal:")
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_from_head() {
        let d = std::env::temp_dir().join(format!("fx-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(".git")).expect("mkdir");
        std::fs::create_dir_all(d.join("src").join("deep")).expect("mkdir");
        std::fs::write(d.join(".git").join("HEAD"), "ref: refs/heads/feature/x\n").expect("w");
        assert_eq!(
            branch(&d.join("src").join("deep")).as_deref(),
            Some("feature/x")
        );
        std::fs::write(d.join(".git").join("HEAD"), "0123456789abcdef\n").expect("w");
        assert_eq!(branch(&d).as_deref(), Some("0123456"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn parses_local_and_remote_only_branches() {
        let out = " \trefs/heads/feature/x\n\
                   *\trefs/heads/main\n\
                   \u{20}\trefs/remotes/origin/HEAD\n\
                   \u{20}\trefs/remotes/origin/main\n\
                   \u{20}\trefs/remotes/origin/fix\n\
                   \u{20}\trefs/remotes/upstream/fix\n";
        let b = parse_refs(out);
        assert_eq!(b.current.as_deref(), Some("main"));
        assert_eq!(b.local, ["feature/x", "main"]);
        assert_eq!(
            b.remote,
            ["origin/fix"],
            "doublons et branches suivies retirés"
        );
        assert_eq!(GitBranches::remote_target("origin/feature/y"), "feature/y");
    }

    #[test]
    fn keeps_the_useful_git_error_line() {
        let e = "error: Your local changes to the following files would be overwritten by checkout:\n\ta.txt\nAborting\n";
        assert!(git_error(e).starts_with("Your local changes"));
        assert_eq!(
            git_error("fatal: invalid reference: x\n"),
            "invalid reference: x"
        );
    }

    #[test]
    fn lists_and_switches_branches_of_a_real_repository() {
        // Sans Git installé (machine de build minimale) : rien à vérifier.
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let d = std::env::temp_dir().join(format!("fx-gitrepo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("mkdir");
        let git = |args: &[&str]| run(&d, args).expect("git");
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(d.join("a.txt"), "a").expect("w");
        git(&["add", "a.txt"]);
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "a",
        ]);
        git(&["branch", "autre"]);
        let b = branches(&d).expect("branches");
        assert_eq!(b.current.as_deref(), Some("main"));
        assert!(b.local.contains(&"autre".to_owned()));
        switch(&d, "autre").expect("switch");
        assert_eq!(branch(&d).as_deref(), Some("autre"));
        assert!(switch(&d, "inexistante").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
