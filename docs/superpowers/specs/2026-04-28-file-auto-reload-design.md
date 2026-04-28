# File auto-reload nei pannelli Editor e Markdown

## Contesto

Pax oggi ha già un meccanismo di reload automatico **parziale**:

- **Pannello Markdown standalone** (`crates/tp-gui/src/panels/markdown.rs`):
  polling mtime ogni 500 ms. In render mode ricarica silenziosamente, in
  edit mode con buffer "dirty" ignora il cambio esterno senza informare
  l'utente.
- **Code Editor — tab "source"** (`crates/tp-gui/src/panels/editor/file_watcher.rs`):
  polling 1 s (file locali) / 5 s (remoti); su conflitto mostra una
  `InfoBar` con due bottoni "Reload / Keep Mine".
- **Code Editor — tab Markdown e Image**: opt-out dal watcher centralizzato
  (`crates/tp-gui/src/panels/editor/tab_content.rs:114-120`). Non si
  aggiornano mai quando il file cambia.

Il risultato è un'esperienza disomogenea: alcune tab si aggiornano,
altre no, e l'edit mode del pannello Markdown perde silenziosamente i
cambi esterni.

## Obiettivo

Comportamento uniforme per tutti i pannelli che mostrano un file:

- **Nessuna modifica locale + file cambiato sul disco** → ricarica automatica e
  silenziosa (comportamento standard ovunque).
- **Modifiche locali non salvate + file cambiato sul disco** → InfoBar
  inline con bottoni **"Ricarica"** e **"Mantieni le mie"**. Stesso pattern
  già usato dalle tab source.

Le tab Image non hanno modifiche locali per definizione, quindi sono
sempre nel ramo "ricarica silenziosa".

## Approccio

Estendere il pattern di watch già esistente. **Nessuna migrazione a
`gio::FileMonitor`**: refactor più ampio fuori scope, può essere un
secondo passo se in futuro il polling diventa un problema.

### 1. Editor — rimuovere l'opt-out di markdown e image tabs

Oggi `start_open_file_watcher` itera solo sulle source tabs. Estendere a
tutti i tipi:

- **Source tab** (comportamento attuale, invariato): polling mtime,
  `show_conflict_bar()` su conflitto, reload silenzioso altrimenti.
- **Markdown tab**: stesso comportamento delle source. Il reload aggiorna
  il `TextBuffer` di edit e ri-renderizza la preview.
- **Image tab**: niente concetto di "modifiche locali" → reload sempre
  silenzioso. Sostituisce l'immagine via `gtk::Picture::set_filename` (o
  equivalente già usato nel viewer immagini).

Implementazione:

- Rimuovere il filtro che esclude markdown/image in
  `crates/tp-gui/src/panels/editor/tab_content.rs:114-120`.
- Discriminare in `file_watcher.rs` per `TabKind` (o per il tipo concreto
  di `TabContent`) e instradare al reload appropriato.
- Esporre, nei moduli che ospitano markdown e image, un metodo
  `reload_from_disk()` chiamabile dal watcher (no parsing diretto del
  file dentro il watcher: la responsabilità sta nel modulo del tab).

### 2. Pannello Markdown standalone — gestione conflitti in edit mode

Stato attuale (`markdown.rs:657-673`): il timer 500 ms ricarica solo in
render mode; in edit-dirty ignora.

Nuovo comportamento:

```text
on tick:
  if mtime cambiata rispetto all'ultima vista:
    if !edit_mode || !dirty:
      reload silenzioso
    else:
      mostra conflict_bar (se non già visibile per questa mtime)
```

UI: aggiungere un campo `conflict_bar: gtk4::InfoBar` al `MarkdownPanel`,
montato in cima al contenitore del pannello (sopra l'area di edit/render).
Bottoni:

- **Ricarica** → rilegge il file, sostituisce il buffer, azzera `modified`,
  aggiorna `last_seen_mtime`, nasconde la bar.
- **Mantieni le mie** → memorizza la mtime corrente come "vista", in modo
  da non riprompare ogni 500 ms; nasconde la bar. La bar riappare solo se
  il file cambia ulteriormente.

Quando l'utente salva da dentro Pax, il `last_seen_mtime` viene
aggiornato → il watcher non mostra l'InfoBar per il salvataggio appena
fatto.

`gtk4::InfoBar` è un widget inline (non un dialog né un popover), quindi
è compatibile con la regola del progetto sui popup custom.

### 3. Niente refactor extra (YAGNI)

- Intervalli di polling lasciati come sono (editor 1 s/5 s, markdown
  panel 500 ms): targettizzano esigenze diverse.
- Nessuna unificazione del watcher tra editor e pannello Markdown
  standalone: vivono in scope diversi (multi-tab vs panel-singolo) e una
  fusione richiederebbe più cambi che valore.
- Nessuna persistenza dello stato "Keep Mine" oltre la sessione del
  pannello: se l'utente chiude e riapre, ripartiamo dallo stato corrente
  del file.

## File coinvolti

- `crates/tp-gui/src/panels/editor/file_watcher.rs` — discriminazione per
  tipo di tab; instradamento a reload silenzioso (image) o conflict-bar
  (markdown, già esistente per source).
- `crates/tp-gui/src/panels/editor/tab_content.rs:114-120` — togliere
  l'opt-out di markdown/image dal tracking del watcher.
- `crates/tp-gui/src/panels/editor/markdown_view.rs` — metodo
  `reload_from_disk(&self, content: &str)` che sostituisce il buffer e
  ri-renderizza.
- `crates/tp-gui/src/panels/editor/image_view.rs` — metodo analogo
  `reload_from_disk(&self, path: &Path)` per la tab Image.
- `crates/tp-gui/src/panels/markdown.rs` — aggiungere `conflict_bar:
  gtk4::InfoBar`, integrarla nel layout in cima al pannello, modificare
  il blocco timer (~657-673) per ramo conflict.

## Verifica end-to-end

1. **Editor source — regressione**: apri `/tmp/x.txt`, da fuori
   `echo nuova > /tmp/x.txt` → la tab si aggiorna entro 1 s. Senza
   modifiche locali, nessuna InfoBar.
2. **Editor source — conflitto** (regressione): apri, modifica senza
   salvare, da fuori cambi il file → InfoBar Reload/Keep Mine; "Reload"
   carica e azzera dirty; "Keep Mine" la nasconde fino al prossimo
   cambio.
3. **Editor markdown — nuovo**: stesso scenario di (1) e (2) su file
   `.md` aperto come tab markdown nell'editor. Sia render che edit
   mode si comportano coerentemente.
4. **Editor image — nuovo**: apri un PNG; rigenera l'immagine dal
   filesystem → la tab mostra la nuova versione senza prompt.
5. **Markdown panel render — regressione**: cambia il file da fuori →
   re-render entro 500 ms.
6. **Markdown panel edit dirty — nuovo**: edit mode con modifiche
   locali; cambia il file da fuori → appare InfoBar; "Ricarica" applica
   il file esterno e azzera dirty; "Mantieni le mie" la nasconde e non
   riappare finché il file non cambia ancora.
7. **Save interno — non-regressione**: salva da Pax stesso → nessuna
   InfoBar (mtime aggiornata da chi ha salvato).
8. **Build**: `cargo build` senza warning nuovi.
