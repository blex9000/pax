"""Pax notebook helpers — auto-injected via PYTHONPATH for cells.

Usage from a cell:

    import pax
    pax.show("/tmp/foo.png")          # render a PNG file inline
    pax.show("data:image/png;base64,...")  # render a base64 PNG inline
    pax.show_plot(plt)                # save a matplotlib figure + show

Import is cheap: this module has no heavy side effects.
"""

import os
import sys
import tempfile


def _emit(line):
    sys.stdout.write(line)
    sys.stdout.write("\n")
    sys.stdout.flush()


def show(target):
    """Render an image inline below the cell.

    `target` is a file path (str/PathLike) or a 'data:image/...' URI.
    """
    target = os.fspath(target) if hasattr(target, "__fspath__") else target
    if not isinstance(target, str):
        raise TypeError("show() expects a str path or data: URI")
    _emit(f"<<pax:image:{target}>>")


def show_plot(plt):
    """Save a matplotlib pyplot/figure to a temp PNG and show it inline.

    `plt` may be the `matplotlib.pyplot` module or a `Figure` instance.
    """
    out_dir = os.environ.get("PAX_OUTPUT_DIR") or tempfile.gettempdir()
    f = tempfile.NamedTemporaryFile(suffix=".png", delete=False, dir=out_dir)
    f.close()
    if hasattr(plt, "savefig"):
        plt.savefig(f.name)
    elif hasattr(plt, "gcf"):
        plt.gcf().savefig(f.name)
    else:
        raise TypeError("show_plot() expects matplotlib.pyplot or a Figure")
    show(f.name)
