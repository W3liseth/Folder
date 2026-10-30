# Notes de version

Changements de chaque version de folder, de la plus récente à la plus ancienne. Ce fichier
est affiché dans Paramètres › Mises à jour › Notes de version et sert de description à la
release GitHub : chaque changement de version dans Cargo.toml commence par une nouvelle
section `## <version> — <date>` en tête (vérifié par `cargo test`).

## 0.8.1 — 2026-10-30

- Mises à jour proposées plus vite : folder vérifie chaque heure (au lieu d'une fois par
  jour), et l'ouverture de Paramètres › Mises à jour revérifie aussitôt si la dernière
  vérification date de plus de 5 minutes. La page n'annonce plus « à jour » d'après une
  vieille vérification.
- « À jour » indique la dernière version publiée : une version tout juste poussée n'est
  proposée qu'une fois sa release publiée, quelques minutes plus tard.

## 0.8.0 — 2026-10-30

- Branche Git dans la barre d'état : dans un dépôt Git, le logo Git et la branche active
  s'affichent en bas à droite. Un clic ouvre la liste des branches (locales, puis
  distantes sans branche locale) et en active une autre ; un filtre apparaît quand il y en
  a beaucoup. Aussi depuis la palette de commandes ou Ctrl + Maj + B (« Changer de branche
  Git »). Nécessite Git pour Windows ; aucun accès au réseau (pas de fetch ni de pull).
- Extraction, renommage, création, copie, déplacement et suppression : le résultat apparaît
  aussitôt dans la liste, même sur les lecteurs qui ne signalent pas leurs changements
  (Google Drive, certains lecteurs réseau) ; plus besoin d'actualiser.
- Renommage qui ne change que les majuscules (« rapport » en « Rapport ») : le nouveau nom
  s'affiche.
- Centre d'état plus grand (plus large et plus haut).

## 0.7.0 — 2026-10-29

- Centre d'état : un bouton à côté de Rechercher et de la palette de commandes remplace
  l'encart en bas à droite. Il montre les copies, déplacements et suppressions en cours
  (progression, temps restant, annulation), puis l'historique des opérations terminées avec
  leur issue, leur heure et leur durée. Un anneau autour du bouton suit la progression, un
  point rouge signale un échec.
- Historique des opérations conservé d'un lancement à l'autre et commun à toutes les
  fenêtres (100 dernières) ; « Effacer l'historique » le vide, une croix retire une ligne.
  Aussi depuis la palette de commandes : « Centre d'état », « Effacer l'historique des
  opérations ».

## 0.6.0 — 2026-10-28

- Mise à jour automatique : folder recherche une nouvelle version au démarrage (au plus une
  fois par jour), la télécharge en arrière-plan et propose de l'installer avec le bouton
  « Mettre à jour » de la barre de titre ; folder redémarre ensuite avec ses onglets.
  Désactivable dans Paramètres › Mises à jour.
- Notes de version : la liste des changements de chaque version, dans Paramètres › Mises à
  jour (aussi depuis la palette de commandes).
- Ouvrir un fichier : le programme démarre dans le dossier du fichier, comme depuis
  l'Explorateur (scripts .bat, .ps1, .py…).

## 0.5.2 — 2026-10-23

- Windows + E sans fenêtre folder ouverte : folder s'ouvre comme depuis son icône, avec les
  onglets de la dernière session ; sinon une nouvelle fenêtre sur l'accueil.

## 0.5.1 — 2026-10-23

- Description plus claire de l'option « Windows + E ouvre folder ».

## 0.5.0 — 2026-10-23

- Option « Windows + E ouvre folder » (Paramètres › Raccourcis › Système) : une nouvelle
  fenêtre par appui ; désactivée, Windows + E rouvre l'Explorateur.
- Raccourci clavier pour ouvrir folder de n'importe où, même fermé.
- L'installeur arrête puis relance le processus de Windows + E lors d'une mise à jour ; la
  désinstallation rend Windows + E à l'Explorateur.

## 0.4.0 — 2026-10-23

- Nouvelle icône.
- Version de l'exécutable visible dans ses propriétés ; version de l'installeur corrigée
  pour les préversions.
- Signature de code préparée (SignPath Foundation).

## 0.3.0 — 2026-10-22

- Menu contextuel : les options du shell sont dessinées par folder (Afficher plus
  d'options, Maj + clic droit), sous-menus Ouvrir avec et Envoyer vers compris.
- Ctrl + C, Ctrl + X et Ctrl + V copient, coupent et collent des fichiers.
- État de synchronisation cloud (OneDrive, Google Drive…) sur les icônes.
- Barre latérale : le surlignage suit le défilement sans retard.

## 0.2.0 — 2026-10-20

- Glisser-déposer de fichiers dans la fenêtre : cible surlignée, aperçu « Déplacer vers » ou
  « Copier vers » sous le pointeur.
- Onglets : réorganisation en direct, détachement dans une nouvelle fenêtre et transfert
  vers une autre fenêtre folder ; seul onglet de sa fenêtre, c'est la fenêtre entière qui
  suit le pointeur.

## 0.1.0 — 2026-10-19

- Première version : gestionnaire de fichiers natif avec rendu CPU, onglets et double
  volet, vues Détails, Liste, Grandes icônes et Colonnes, volet d'aperçu, étiquettes,
  recherche, intégration au shell Windows, fenêtres Paramètres et Propriétés, installeur.
