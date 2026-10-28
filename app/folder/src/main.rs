// Pas de console en release ; en debug, la console sert aux journaux.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use fx_ui::{BenchMode, Renderer};

const USAGE: &str = "\
Usage : folder [OPTIONS] [DOSSIER]

Options :
  --renderer cpu|glow|wgpu   moteur de rendu (défaut : FOLDER_RENDERER, sinon cpu)
  --bench-nav DOSSIER        mesures de démarrage et de navigation (lignes FX_BENCH sur stdout)
  --bench-scroll DOSSIER     mesure des temps d'image en défilement continu
  --size LxH                 taille intérieure visée en pixels physiques (mesures)
  --rect G,H,D,B             placement de la fenêtre en pixels physiques (onglet détaché)

Variables : FOLDER_LOG=<filtre tracing> (défaut : warn), FOLDER_RENDERER";

fn main() -> ExitCode {
    let process_start = Instant::now();
    init_logging();

    // Ouverture d'une image ou d'une vidéo avec ses voisins : processus éphémère.
    let raw_os: Vec<OsString> = std::env::args_os().skip(1).collect();
    if raw_os.first().is_some_and(|a| a == "--open-media") {
        return if fx_ui::open_media(&raw_os[1..]) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }

    // Processus résident de Windows + E (option des réglages « Raccourcis »).
    if raw_os.first().is_some_and(|a| a == fx_ui::WIN_E_ARG) {
        return if fx_ui::run_win_e() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }

    // Recherche de mise à jour (lancée par une fenêtre folder) : réponse sur stdout.
    if raw_os.first().is_some_and(|a| a == fx_ui::UPDATE_CHECK_ARG) {
        println!("{}", fx_ui::update_check());
        return ExitCode::SUCCESS;
    }

    // Mise à jour (installeur) : processus résident arrêté, option inchangée ; relancé ensuite.
    if raw_os.first().is_some_and(|a| a == "--stop-win-e") {
        fx_ui::stop_win_e();
        return ExitCode::SUCCESS;
    }

    // Désinstallation : Windows + E rendu à l'Explorateur, raccourci d'ouverture retiré.
    if raw_os.first().is_some_and(|a| a == "--restore-explorer") {
        fx_ui::restore_explorer();
        return ExitCode::SUCCESS;
    }

    // Fenêtre Propriétés (lancée par la fenêtre principale) : processus à part, le temps de
    // la fenêtre.
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("--settings") {
        let owner = raw
            .iter()
            .position(|a| a == "--owner")
            .and_then(|i| raw.get(i + 1))
            .and_then(|v| v.parse().ok());
        let notes = raw.iter().any(|a| a == "--notes");
        return match fx_ui::settings_window::run(owner, notes) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("folder : {e}");
                ExitCode::FAILURE
            }
        };
    }
    if raw.first().map(String::as_str) == Some("--properties") {
        return match fx_ui::props_window::parse_args(&raw[1..]) {
            Ok(args) => match fx_ui::props_window::run(args) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("folder : {e}");
                    ExitCode::FAILURE
                }
            },
            Err(e) => {
                eprintln!("folder --properties : {e}");
                ExitCode::from(2)
            }
        };
    }

    let args = match parse_args(std::env::args_os().skip(1)) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let opts = fx_ui::Options {
        start_dir: args.dir,
        bench: args.bench,
        process_start,
        renderer: args.renderer.unwrap_or_else(Renderer::from_env),
        window_px: args.size,
        rect: args.rect,
    };
    match fx_ui::run(opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "arrêt sur erreur");
            eprintln!("folder : {e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, PartialEq)]
struct Args {
    dir: Option<PathBuf>,
    bench: BenchMode,
    renderer: Option<Renderer>,
    size: Option<[u32; 2]>,
    rect: Option<[i32; 4]>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            dir: None,
            bench: BenchMode::Off,
            renderer: None,
            size: None,
            rect: None,
        }
    }
}

fn parse_args(mut it: impl Iterator<Item = OsString>) -> Result<Args, String> {
    let mut args = Args::default();
    while let Some(a) = it.next() {
        let mut value = |flag: &str| it.next().ok_or(format!("{flag} attend une valeur"));
        match a.to_str() {
            Some("-h" | "--help") => return Err("folder — gestionnaire de fichiers".into()),
            Some(flag @ ("--bench-nav" | "--bench-scroll")) => {
                args.bench = if flag == "--bench-nav" {
                    BenchMode::Nav
                } else {
                    BenchMode::Scroll
                };
                args.dir = Some(value(flag)?.into());
            }
            Some("--renderer") => {
                let v = value("--renderer")?;
                let name = v.to_string_lossy();
                args.renderer =
                    Some(Renderer::parse(&name).ok_or(format!("moteur inconnu : {name}"))?);
            }
            Some("--size") => {
                let v = value("--size")?.to_string_lossy().into_owned();
                let (w, h) = v.split_once(['x', 'X']).ok_or("--size attend LxH")?;
                let px = |s: &str| {
                    s.parse::<u32>()
                        .map_err(|_| format!("taille invalide : {v}"))
                };
                args.size = Some([px(w)?, px(h)?]);
            }
            Some("--rect") => {
                let v = value("--rect")?.to_string_lossy().into_owned();
                let n: Vec<i32> = v
                    .split(',')
                    .map(|s| s.trim().parse::<i32>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| format!("placement invalide : {v}"))?;
                match n[..] {
                    [l, t, r, b] if r > l && b > t => args.rect = Some([l, t, r, b]),
                    _ => return Err(format!("--rect attend G,H,D,B : {v}")),
                }
            }
            Some(s) if s.starts_with("--") => return Err(format!("option inconnue : {s}")),
            _ if args.dir.is_none() => args.dir = Some(a.into()),
            _ => return Err("un seul dossier attendu".into()),
        }
    }
    Ok(args)
}

fn init_logging() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("FOLDER_LOG").unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: &[&str]) -> Result<Args, String> {
        parse_args(v.iter().map(OsString::from))
    }

    #[test]
    fn args() {
        assert_eq!(parse(&[]), Ok(Args::default()));
        assert_eq!(parse(&[r"C:\x"]).map(|a| a.dir), Ok(Some(r"C:\x".into())));
        let a = parse(&[
            "--bench-scroll",
            "d",
            "--renderer",
            "cpu",
            "--size",
            "1920x1080",
        ])
        .expect("args");
        assert_eq!(a.bench, BenchMode::Scroll);
        assert_eq!(a.renderer, Some(Renderer::Cpu));
        assert_eq!(a.size, Some([1920, 1080]));
        assert_eq!(
            parse(&["--bench-nav", "d"]).map(|a| a.bench),
            Ok(BenchMode::Nav)
        );
        assert!(parse(&["--bench-nav"]).is_err());
        assert!(parse(&["--renderer", "vulkan"]).is_err());
        assert!(parse(&["--size", "big"]).is_err());
        assert_eq!(
            parse(&["--rect", "-10,20,1190,800", "folder:home"]).map(|a| (a.rect, a.dir)),
            Ok((Some([-10, 20, 1190, 800]), Some("folder:home".into())))
        );
        assert!(parse(&["--rect", "1,2,3"]).is_err());
        assert!(parse(&["--rect", "10,10,5,20"]).is_err());
        assert!(parse(&["--nope"]).is_err());
    }
}
