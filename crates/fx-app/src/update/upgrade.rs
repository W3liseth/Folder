//! Mise à jour : états et annonces (le travail est dans `crate::upgrade`, hors thread UI).

use std::time::Duration;

use crate::effect::Effect;
use crate::state::{AppState, ToastKind};
use crate::upgrade::Status;

/// Délai de la vérification automatique : le démarrage passe d'abord.
const BACKGROUND_DELAY: Duration = Duration::from_secs(5);

pub(crate) fn check(state: &mut AppState, force: bool) -> Vec<Effect> {
    if force {
        state.toast("Recherche des mises à jour…", ToastKind::Info);
        // Une mise à jour déjà prête reste installable pendant la recherche.
        if state.update.ready_version().is_none() {
            state.update = Status::Checking;
        }
    }
    vec![Effect::CheckUpdate {
        force,
        delay: if force {
            Duration::ZERO
        } else {
            BACKGROUND_DELAY
        },
    }]
}

/// Résultat d'une vérification. Une vérification automatique n'annonce qu'une version
/// nouvellement téléchargée et n'efface pas une mise à jour prête si elle échoue (réseau
/// absent) ; une demande explicite annonce tout.
pub(crate) fn checked(state: &mut AppState, status: Status, fresh: bool, force: bool) {
    let was_ready = state.update.ready_version().map(str::to_owned);
    match &status {
        Status::Ready { version, .. }
            if force || (fresh && was_ready.as_ref() != Some(version)) =>
        {
            state.toast(
                format!("folder {version} est prêt : « Mettre à jour » l'installe."),
                ToastKind::Info,
            );
        }
        Status::UpToDate { latest, .. } if force => {
            state.toast(
                format!(
                    "folder {} est à jour (dernière version publiée : {latest}).",
                    env!("CARGO_PKG_VERSION")
                ),
                ToastKind::Success,
            );
        }
        Status::NotInstalled if force => state.toast(
            "Mise à jour indisponible : folder n'a pas été installé par son installeur.",
            ToastKind::Error,
        ),
        Status::Failed(e) if force => {
            state.toast(format!("Mise à jour : {e}"), ToastKind::Error);
        }
        Status::Failed(_) if was_ready.is_some() => return,
        _ => {}
    }
    state.update = status;
}

/// L'installeur est relu dans le fichier partagé : la demande peut venir des Paramètres,
/// qui l'ont téléchargé eux-mêmes.
pub(crate) fn install(_state: &mut AppState) -> Vec<Effect> {
    vec![Effect::InstallUpdate]
}
