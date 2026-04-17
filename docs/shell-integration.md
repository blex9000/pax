# Shell integration (OSC 0 / OSC 7 / OSC 133)

Pax inietta automaticamente hook nella shell di ogni panel terminale per
pilotare gli indicatori nell'header del panel e nelle tab label. Questo
documento descrive cosa viene iniettato, perché, e come disabilitarlo o
personalizzarlo.

## Cosa viene iniettato

Appena la shell è pronta (dopo `.bashrc`), pax esegue un blocco di comandi
di setup racchiuso tra `set +o history` e `set -o history` — l'history
della shell non viene contaminata.

**VTE backend (Linux, default)**, in `crates/tp-gui/src/panels/terminal/vte_backend.rs`:

```bash
set +o history
export PS1='\[\033[32m\]$:\[\033[0m\] '              # prompt minimalista
__pax_prompt() {
    local d="${PWD/#$HOME/~}"
    printf '\033]0;%s@%s: %s\007' "$USER" "$HOSTNAME" "$d"   # OSC 0: titolo finestra
    printf '\033]7;file://%s%s\033\\' "$HOSTNAME" "$PWD"     # OSC 7: directory URI (footer)
    printf '\033]133;A\007'                                   # OSC 133;A: prompt pronto
    __pax_preexec_fired=
}
__pax_preexec() {
    [[ -n "$__pax_preexec_fired" ]] && return
    __pax_preexec_fired=1
    printf '\033]133;C\007'                                   # OSC 133;C: comando in esecuzione
}
PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }__pax_prompt"
trap '__pax_preexec' DEBUG
set -o history
```

**PTY backend (macOS, `--no-default-features`)**, in `crates/tp-gui/src/panels/terminal/pty_backend.rs`:
stesso payload meno PS1 (la shell mantiene il prompt utente) e meno OSC 7
(il footer non è tracciato sul fallback PTY).

## A cosa serve ciascun OSC

| Sequenza | Scopo |
|---|---|
| `ESC]0;<title>\007` | Titolo finestra/tab. Pax lo mostra centrato nell'header del panel. |
| `ESC]7;file://<host><pwd>\033\\` | URI della working directory. Pax lo mostra nel footer (solo VTE). |
| `ESC]133;A\007` | "Prompt pronto" (shell idle). Pax **spegne** l'indicatore attività. |
| `ESC]133;C\007` | "Comando in esecuzione". Pax **accende** l'indicatore ambra nell'header e sulle tab label padri. |

Il flag `__pax_preexec_fired` previene re-emissioni di OSC 133;C per
ogni comando della pipeline — il segnale scatta solo sulla **prima**
esecuzione dopo ogni prompt e viene azzerato da `__pax_prompt` al ciclo
successivo.

## Perché appendere a `PROMPT_COMMAND` anziché sovrascrivere

Pre-fix (master ~`3ea7479^`) pax sostituiva completamente `PROMPT_COMMAND`,
distruggendo eventuali hook utente (git prompt, agenti SSH, ecc.). Ora
`PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }__pax_prompt"` appende
la funzione pax dopo l'eventuale `PROMPT_COMMAND` ereditato da `.bashrc`,
preservando il comportamento utente.

## Compatibilità VTE

Le signal `shell-precmd` / `shell-preexec` (binding VTE per OSC 133;A/C)
sono state aggiunte in **VTE 0.80** (`glib::signal::connect_raw`:
`assertion failed: handle > 0` su versioni precedenti). Pax rileva la
versione runtime via `vte4::ffi::vte_get_minor_version()` e salta la
connessione sui runtime < 0.80. L'indicatore resta inerte su quei
sistemi, ma non c'è crash.

Per il PTY backend il parser `vte-0.11` (usato da `alacritty_terminal`)
**non dispatcha** OSC 133 → il reader thread fa uno scanning bytewise
diretto sullo stream PTY con un carry buffer di 8 byte per non perdere
sequenze che si spezzano fra due read successivi.

## Disabilitare l'integrazione

Non è attualmente esposta una setting di opt-out. Workaround utente:
mettere in `.bashrc` righe che ripuliscono gli hook:

```bash
unset -f __pax_prompt __pax_preexec 2>/dev/null
trap - DEBUG
PROMPT_COMMAND="${PROMPT_COMMAND//__pax_prompt/}"
```

Chiaramente sconsigliato — meglio aprire una issue se c'è bisogno di
disattivarla in modo ufficiale.

## Shell diverse da bash

Tutto quanto sopra assume **bash**. Lo script usa `local`,
`${PWD/#$HOME/~}`, `[[ ... ]]`, `trap DEBUG` — sintassi bash.
`zsh` accetta la maggior parte ma `trap DEBUG` funziona diversamente
(`preexec` hook nativo). `fish` non funziona.

Se pax viene lanciato con `SHELL=/usr/bin/fish` o altre shell non-bash,
l'iniezione va a vuoto silenziosamente (sintassi non valida → shell
segnala errore a stderr ma non crasha). Gli indicatori e il titolo OSC
restano vuoti.
