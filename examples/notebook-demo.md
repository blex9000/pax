# Pax Notebook — demo

---

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
print("#ciao")
```

## Bash — output markdown multilinea

```bash run
cat <<'EOF'
### Disk usage
| mount | size | used |
|-------|------|------|
| `/`   | 256G | 64G  |
| `/home` | 1T | 320G |

**Note**:

- Il *block* è renderizzato dal renderer markdown del pannello.
- Le **emoji** funzionano: ✅ ⚠️ 🚀
- Anche le `inline code spans`.

> Citazione: "anything that prints to stdout becomes markdown".
EOF
```

## sh — lista numerata + link

```sh run
printf "1. Primo\n"
printf "2. Secondo con [link](https://example.com)\n"
printf "3. Terzo: ~~strikethrough~~ e **bold**\n"
printf "\n---\n\n"
printf "Riga finale dopo separatore.\n"
```

## Python — output markdown multilinea

```python run
print("### Sistema")
print()
print("| chiave | valore |")
print("|--------|--------|")
import sys, platform
print(f"| python | `{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}` |")
print(f"| os | `{platform.system()} {platform.release()}` |")
print(f"| arch | `{platform.machine()}` |")
print()
print("#### Lista")
for n in range(1, 4):
    print(f"- elemento **{n}** — `2^{n}` = `{2**n}`")
print()
print("> Tutto markdown, nessun pre-processing.")
```

## Python — heading + code-fence dentro l'output

```python run
print("## Snippet generato")
print()
print("```rust")
print("fn main() {")
print("    println!(\"hello\");")
print("}")
print("```")
print()
print("Il code-fence è preservato come blocco verbatim.")
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
