# egui_software_backend (copie vendue)

- Origine : <https://github.com/DGriffin91/egui_software_backend>, crate 0.0.3 (crates.io).
- Licence : MIT OR Apache-2.0 (fichiers `LICENSE-MIT` et `LICENSE-APACHE` conservés).
- Pourquoi vendue : la version publiée dépend d'egui 0.34 ; fx utilise egui 0.37.
- Modifications : dépendances egui/egui-winit passées en 0.37 et adaptations d'API
  correspondantes ; module `test_render` (egui_kittest) retiré ; exemples et tests retirés.
- Code tiers : il contient de l'`unsafe` (SIMD, accès tampon). C'est la seule exception à la
  règle « `unsafe` uniquement dans `fx-win` » (cf. CLAUDE.md) ; ne pas y ajouter de code fx.
- Fenêtres sans décorations (portage fx) : `BeginResize`, `StartDrag`, `IMERect`, `IMEAllowed`
  et appel à `egui_winit::update_viewport_info` (état agrandi, focus) implémentés.
- `TexturesDelta` vidé après application dans la boucle `winit` (portage fx) : egui 0.37
  déclenche sinon une assertion en debug à la destruction du delta.
- Effacement optionnel (portage fx) : `set_clear_each_frame(false)` saute le `fill(0)` du
  tampon avant chaque image ; fx l'utilise car ses panneaux opaques couvrent toute la fenêtre.
- Rendu partiel et défilement par décalage (portage fx, `src/damage.rs`, nouvelle méthode
  `EguiSoftwareRender::render_damaged`, utilisée par la boucle `winit` quand le cache est
  désactivé) :
  - la liste de primitives est comparée à celle de l'image précédente (empreintes absolue et
    relative au premier sommet, appariement croissant) ; seules les zones modifiées (tuiles de
    16×8 px) sont rasterisées, puis présentées par `present_with_damage` (en bandes de 64 px
    pour limiter le nombre de `BitBlt`) ;
  - un contenu translaté verticalement d'un nombre entier de pixels dans une découpe restée en
    place (liste qui défile) est détecté ; ses pixels sont recopiés (`shift_rects`), seulement
    là où du contenu mobile se trouvait ou arrive, et seule la bande découverte est rasterisée.
    Les primitives fixes qui recouvrent la zone (fond de la carte, ombres de bord) sont
    analysées triangle par triangle : intérieur des aplats unis invariant, bords et triangles
    non unis endommagés ;
  - mises à jour de texture : seules les primitives qui échantillonnent la zone réécrite
    (nouveaux glyphes de l'atlas) sont redessinées ;
  - repli sur un rendu complet : premier rendu, tampon neuf (`age() != 1`), taille ou échelle
    changées, redimensionnement, restauration, changement de thème, plus de 60 % de tuiles
    touchées ; présentation complète après un déplacement de la fenêtre ;
  - `set_partial_redraw(false)` revient au rendu complet de chaque image (fx :
    `FOLDER_CPU_FULL_REDRAW=1`) ;
  - `draw_egui_mesh` écarte d'emblée les triangles hors de la découpe (les passes par
    rectangle en profitent ; sans effet sur l'image) ;
  - écart avec un rendu complet : au plus 2 niveaux par canal sur les primitives texturées
    coupées par un rectangle (l'interpolation des UV part du bord découpé) ; vérifié image par
    image par `renderbench --verify` et par le test `partial_render_matches_full_render`.
- Images différées (portage fx) : l'original redessinait aussitôt toute demande
  `request_repaint_after`, puis attendait ; la boucle tournait donc à vide jusqu'à l'échéance.
  L'échéance est désormais retenue (`next_repaint`) et la boucle attend jusqu'à elle.
- Cadence (portage fx) : `set_frame_pacing(true)` limite les images demandées par egui
  (animations) à une par rafraîchissement de l'écran (fréquence lue sur le moniteur de la
  fenêtre, 60 Hz par défaut) ; les entrées restent immédiates. folder la désactive pendant le
  banc de défilement, qui mesure le débit sans plafond.
- Fermeture (portage fx) : `App::on_exit` est appelé une seule fois, aussi quand la fenêtre se
  ferme par `ViewportCommand::Close` (l'original ne l'appelait que pour la fermeture native ;
  la session de folder n'était donc pas sauvegardée par le bouton de sa barre de titre).
- Raccourcis du presse-papiers (portage fx) : `set_clipboard_shortcuts(false)` laisse
  Ctrl+C/X/V (et Ctrl+Inser, Maj+Inser, Maj+Suppr) arriver comme des touches ordinaires au
  lieu des événements `Copy`/`Cut`/`Paste` d'egui-winit. Sans cela, folder ne voyait jamais
  ces touches, et Ctrl+V ne produisait rien du tout quand le presse-papiers ne contenait que
  des fichiers (pas de texte).
- Fenêtre créée cachée (portage fx) : Windows n'envoie pas de WM_PAINT à une fenêtre
  invisible ; la première image est donc produite dès la création de la surface, ce qui
  permet à folder de placer la fenêtre (écran, taille, état agrandi) puis de l'afficher
  directement ainsi.
