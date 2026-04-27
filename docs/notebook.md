# Markdown Notebook

Il pannello Markdown di Pax esegue inline i fenced code blocks marcati con
un tag eseguibile, mostrando l'output (testo + immagini) sotto al blocco.
Modello "leggero": ogni blocco è un subprocess isolato, niente kernel
persistenti, niente stato condiviso tra blocchi. L'output vive solo in
memoria — chiudere il pannello lo scarta.

## Sintassi tag

````
```python run                 ← una sola esecuzione, manuale (pulsante ▶)
```python once                ← alias di `run`
```bash watch=5s              ← ciclico ogni 5 secondi (auto-start se visibile)
```sh watch=2m                ← ciclico ogni 2 minuti
```python run timeout=120s    ← override del wall-clock cap
```python watch=2s confirm    ← chiedi conferma alla prima esecuzione
````

Linguaggi: `python`, `bash`, `sh`. `python` risolve `python3` poi `python`
in PATH.

Un blocco fenced senza tag (es. solo `python`) viene renderizzato come
codice statico — non si esegue.

## Output ricco (immagini, plot)

Per emettere un'immagine inline, usa il marker stdout:

````
```python run
print("<<pax:image:/tmp/plot.png>>")
```
````

Da Python è più comodo l'helper auto-iniettato:

````
```python run
import pax, matplotlib.pyplot as plt
plt.plot([1,2,3])
pax.show_plot(plt)            # salva PNG temp ed emette il marker
pax.show("/tmp/static.png")   # solo file path
```
````

In v1 le immagini su file sono renderizzate inline; le `data:image/...` URI
sono visualizzate come warning (decoding base64 da implementare).

## Lifecycle watch

- `watch=Ns` parte automaticamente al primo render, in modo non bloccante.
- Si mette in pausa quando il pannello non è visibile (tab cambio,
  pannello chiuso).
- In v1, **passare a Edit mode e tornare a Render rebuilda l'engine**:
  l'output finora accumulato viene scartato e i `watch` ripartono da
  zero. Stesso comportamento per il pulsante ↻ (reload) e per modifiche
  esterne al file. (Una iterazione futura potrà preservare l'engine
  attraverso il toggle.)
- Per le celle marcate `confirm`, l'auto-start del watch è bloccato
  finché l'utente non clicca ▶ almeno una volta — questo è il safety
  net del tag.
- Salta un tick se il run precedente è ancora vivo (no accodamento).
- Chiusura pannello → SIGTERM al subprocess, SIGKILL dopo 2s.

## Pannello standalone vs editor

L'esecuzione delle celle notebook è abilitata solo nel pannello Markdown
standalone (apri un `.md` come pannello principale). La tab Markdown del
Code Editor renderizza i tag come blocchi codice statici senza eseguirli.

## Sicurezza

Una blocklist minima impedisce comandi distruttivi ovvi (`rm -rf /`,
`mkfs`, fork bomb, `shutdown`, …) sulle celle shell (`bash`/`sh`). I cell
Python non sono filtrati dalla blocklist (i pattern shell genererebbero
falsi positivi su nomi di metodo come `executor.shutdown(...)`).

Per il resto i cell girano con i tuoi privilegi: **non aprire notebook
scaricati da fonti non fidate**. Il tag `confirm` è un placeholder per un
veto manuale (in v1 il dialog è uno stub e accetta sempre — il tag resta
utile come dichiarazione di intento, una vera UI di conferma arriva in
una iterazione successiva).

## Limiti operativi

- Max 8 processi notebook attivi per processo Pax.
- Default timeout: 30s per `run`/`once`. `watch` non ha un timeout (il
  tick successivo sostituisce il precedente solo se è già finito).

## Troubleshooting

- "blocked: …" → la blocklist ha intercettato il codice (solo bash/sh).
  Riformula il comando o estendi `tp-core/src/safety.rs::notebook_blocklist()`.
- "python interpreter not found" → installa `python3` e assicurati che
  sia in PATH del processo Pax.
- Immagine non si vede → controlla che il path sia assoluto ed esista
  al momento dell'esecuzione (i path relativi non sono risolti).
