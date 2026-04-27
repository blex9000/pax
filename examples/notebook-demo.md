# Pax Notebook — demo

Apri questo file nel pannello Markdown di Pax in modalità Render.

## Bash one-shot

```bash run
echo "## Hello from bash"
date
```

## Python one-shot

```python run
import sys
print(f"Python {sys.version}")
print("ciao")
```

## Watch — un orologio ogni 1s

```bash watch=2s
date '+%H:%M:%S'
```

## Watch con conferma — opt-in

```python watch=2s confirm
import random
print(f"random = {random.random():.4f}")
```

## Plot inline (richiede matplotlib)

> **Pre-requisito**: `pip install --user matplotlib` (oppure equivalente
> nel tuo venv/pipx). Senza il pacchetto la cella restituisce
> `ModuleNotFoundError: No module named 'matplotlib'`.

```python run
import pax
try:
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
except ModuleNotFoundError:
    print("matplotlib not installed — skip this cell or install with `pip install --user matplotlib`.")
else:
    plt.figure()
    plt.plot([1, 2, 4, 8, 16])
    plt.title("powers of 2")
    pax.show_plot(plt)
```

## Bloccato dalla blocklist

```bash run
rm -rf /
```
