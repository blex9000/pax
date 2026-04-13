from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont


ROOT = Path(__file__).resolve().parent.parent
BASE_IMAGE = ROOT / "current-graphite.png"
OUT_DIR = ROOT / "mockups"

W, H = Image.open(BASE_IMAGE).size


def font(size: int, bold: bool = False) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    candidates = []
    if bold:
        candidates.extend(
            [
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/truetype/liberation2/LiberationSans-Bold.ttf",
            ]
        )
    else:
        candidates.extend(
            [
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf",
            ]
        )
    for path in candidates:
        try:
            return ImageFont.truetype(path, size=size)
        except OSError:
            continue
    return ImageFont.load_default()


F10 = font(10)
F12 = font(12)
F14 = font(14)
F16 = font(16)
F18B = font(18, bold=True)
F24B = font(24, bold=True)


BG = (17, 22, 29, 255)
SURFACE = (28, 34, 44, 242)
SURFACE_2 = (35, 42, 54, 242)
SURFACE_3 = (22, 27, 36, 235)
BORDER = (80, 94, 110, 130)
TEXT = (217, 225, 234, 255)
MUTED = (136, 146, 160, 255)
ACCENT = (85, 134, 235, 255)
ACCENT_SOFT = (85, 134, 235, 65)
GOOD = (88, 203, 134, 255)
WARNING = (242, 201, 76, 255)


def round_rect(draw: ImageDraw.ImageDraw, box, fill, outline=None, radius=12, width=1):
    draw.rounded_rectangle(box, radius=radius, fill=fill, outline=outline, width=width)


def add_top_badge(img: Image.Image, title: str, subtitle: str):
    draw = ImageDraw.Draw(img)
    x, y = 28, 26
    round_rect(draw, (x, y, x + 410, y + 72), (12, 15, 21, 210), (255, 255, 255, 28), radius=18)
    draw.text((x + 18, y + 14), title, font=F24B, fill=TEXT)
    draw.text((x + 18, y + 44), subtitle, font=F12, fill=MUTED)


def add_footer_note(img: Image.Image, text: str):
    draw = ImageDraw.Draw(img)
    box_w = 560
    x = W - box_w - 26
    y = H - 54
    round_rect(draw, (x, y, x + box_w, y + 30), (12, 15, 21, 190), radius=14)
    draw.text((x + 14, y + 8), text, font=F10, fill=MUTED)


def chip(draw: ImageDraw.ImageDraw, x: int, y: int, text: str, tone="muted", compact=False):
    tone_map = {
        "active": ((67, 101, 177, 235), TEXT),
        "muted": ((31, 37, 47, 210), MUTED),
        "soft": ((24, 30, 39, 190), (170, 180, 192, 255)),
        "good": ((32, 63, 45, 235), GOOD),
        "warn": ((67, 56, 24, 235), WARNING),
    }
    fill, fg = tone_map[tone]
    pad_x = 11 if not compact else 8
    pad_y = 6 if not compact else 4
    font_used = F12 if not compact else F10
    tw = draw.textbbox((0, 0), text, font=font_used)[2]
    w = tw + pad_x * 2
    h = 24 if not compact else 18
    round_rect(draw, (x, y, x + w, y + h), fill, radius=10 if not compact else 8)
    draw.text((x + pad_x, y + (5 if not compact else 3)), text, font=font_used, fill=fg)
    return x + w + 8


def tab(draw: ImageDraw.ImageDraw, x: int, y: int, text: str, active=False, compact=False):
    pad_x = 14 if not compact else 10
    pad_y = 7 if not compact else 5
    tw = draw.textbbox((0, 0), text, font=F14 if not compact else F12)[2]
    w = tw + pad_x * 2
    h = 28 if not compact else 22
    fill = (56, 70, 92, 240) if active else (0, 0, 0, 0)
    outline = ACCENT if active else None
    round_rect(draw, (x, y, x + w, y + h), fill, outline, radius=10)
    draw.text(
        (x + pad_x, y + (6 if not compact else 4)),
        text,
        font=F14 if not compact else F12,
        fill=TEXT if active else MUTED,
    )
    return x + w + 8


def line(draw: ImageDraw.ImageDraw, xy, fill, width=1):
    draw.line(xy, fill=fill, width=width)


def soften_area(img: Image.Image, box, tint, blur=0):
    overlay = Image.new("RGBA", img.size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(overlay)
    draw.rectangle(box, fill=tint)
    if blur:
        overlay = overlay.filter(ImageFilter.GaussianBlur(blur))
    return Image.alpha_composite(img, overlay)


def panel_surface(draw: ImageDraw.ImageDraw, box, title=None, subtitle=None, active=False, radius=14):
    fill = (15, 19, 25, 242) if active else (16, 21, 28, 226)
    outline = (83, 134, 235, 70) if active else (255, 255, 255, 18)
    round_rect(draw, box, fill, outline=outline, radius=radius)
    if title:
        x1, y1, _, _ = box
        draw.text((x1 + 18, y1 + 14), title, font=F16, fill=TEXT if active else (198, 206, 215, 235))
        if subtitle:
            draw.text((x1 + 18, y1 + 37), subtitle, font=F10, fill=MUTED)


def top_canvas_hint(draw: ImageDraw.ImageDraw, x: int, y: int, label: str):
    round_rect(draw, (x, y, x + 248, y + 24), (11, 15, 20, 180), radius=10)
    draw.text((x + 12, y + 6), label, font=F10, fill=MUTED)


def draw_code_lines(draw: ImageDraw.ImageDraw, x: int, y: int, width: int, rows: int = 12, tint=TEXT):
    colors = [
        (120, 172, 255, 220),
        (196, 167, 231, 220),
        (136, 192, 208, 220),
        (163, 190, 140, 220),
        (216, 222, 233, 200),
    ]
    for i in range(rows):
        yy = y + i * 18
        ln = x + 8
        line_w = max(80, int(width * (0.35 + (i % 5) * 0.12)))
        draw.text((x - 26, yy), f"{i+1:>2}", font=F10, fill=(93, 104, 118, 180))
        line(draw, (ln, yy + 8, ln + line_w, yy + 8), colors[i % len(colors)], 2)
        if i % 3 == 1:
            line(draw, (ln + line_w + 12, yy + 8, ln + line_w + 90, yy + 8), (110, 120, 132, 120), 2)


def draw_terminal_lines(draw: ImageDraw.ImageDraw, x: int, y: int, lines: list[tuple[str, tuple[int, int, int, int]]]):
    for idx, (text, fill) in enumerate(lines):
        draw.text((x, y + idx * 22), text, font=F14, fill=fill)


def app_shell():
    img = Image.new("RGBA", (W, H), BG)
    draw = ImageDraw.Draw(img)
    draw.rectangle((0, 0, W, H), fill=BG)
    draw.rectangle((0, 0, W, 34), fill=(30, 36, 47, 255))
    draw.text((22, 10), "≡", font=F16, fill=MUTED)
    draw.text((W // 2 - 90, 10), "Pax  —  Workspaces", font=F12, fill=MUTED)
    for i, sym in enumerate(["▢", "○", "•", "×"]):
        draw.text((W - 122 + i * 28, 10), sym, font=F12, fill=MUTED)
    return img


def draw_sidebar(draw: ImageDraw.ImageDraw, modern=False):
    x2 = 196 if modern else 202
    fill = (27, 33, 43, 255) if modern else (23, 29, 38, 255)
    draw.rectangle((0, 96, x2, H), fill=fill)
    draw.rectangle((0, 72, x2, 96), fill=(24, 30, 39, 255))
    draw.text((18, 80), "freeflow-web", font=F10, fill=TEXT)
    draw.text((16, 118), "Files   Search   Git", font=F12, fill=MUTED if modern else TEXT)
    entries = [
        ".claude",
        "plans",
        ".git",
        "branches",
        "hooks",
        "objects",
        "playwright-mcp",
        "certs",
    ]
    yy = 156
    for name in entries:
        draw.text((22, yy), f"▸ {name}", font=F12, fill=TEXT if not modern else (202, 210, 220, 255))
        yy += 28


def draw_root_tabs(draw: ImageDraw.ImageDraw, style="default"):
    if style == "minimal":
        draw.text((28, 48), "freeflow", font=F14, fill=TEXT)
        draw.text((126, 48), "deploy", font=F14, fill=MUTED)
        line(draw, (26, 68, 92, 68), ACCENT, 3)
    else:
        round_rect(draw, (18, 40, 306, 72), SURFACE_3, outline=(255, 255, 255, 14), radius=14)
        x = 32
        x = tab(draw, x, 46, "freeflow", active=True, compact=True)
        tab(draw, x, 46, "deploy", active=False, compact=True)


def draw_editor_surface(draw: ImageDraw.ImageDraw, box, title, modern=False, unified=False):
    x1, y1, x2, y2 = box
    fill = (16, 20, 27, 255) if modern else (17, 21, 28, 255)
    outline = (255, 255, 255, 18) if modern else (83, 134, 235, 64)
    radius = 16 if modern else 14
    round_rect(draw, box, fill, outline=outline, radius=radius)
    if unified:
        draw.text((x1 + 18, y1 + 14), title, font=F10, fill=MUTED)
    else:
        draw.text((x1 + 18, y1 + 14), title, font=F16, fill=TEXT)
        draw.text((x1 + 18, y1 + 38), "code editor", font=F10, fill=MUTED)
    draw_code_lines(draw, x1 + 36, y1 + 86, x2 - x1 - 120, rows=14)


def draw_markdown_panel(draw: ImageDraw.ImageDraw, box, modern=False, unified=False):
    x1, y1, x2, y2 = box
    round_rect(draw, box, (16, 20, 27, 255), outline=(255, 255, 255, 18), radius=14)
    draw.text((x1 + 18, y1 + 14), "markdown", font=F10 if unified else F16, fill=MUTED if unified else TEXT)
    if not unified:
        draw.text((x1 + 18, y1 + 38), "preview refresh", font=F12, fill=MUTED)
    draw.text((x1 + 24, y1 + 86), "# Release notes", font=F16, fill=TEXT)
    draw.text((x1 + 24, y1 + 118), "- Deploy checklist", font=F14, fill=MUTED)
    draw.text((x1 + 24, y1 + 146), "- Links to dashboards", font=F14, fill=MUTED)
    draw.text((x1 + 24, y1 + 174), "- TODO items", font=F14, fill=MUTED)


def draw_terminal_panel(draw: ImageDraw.ImageDraw, box, title="terminal", modern=False, compact=False):
    x1, y1, x2, y2 = box
    round_rect(draw, box, (15, 18, 24, 255), outline=(255, 255, 255, 18), radius=14)
    draw.text((x1 + 18, y1 + 14), title, font=F10 if compact else F16, fill=MUTED if compact else TEXT)
    if not compact:
        draw.text((x1 + 18, y1 + 38), "local shell", font=F10, fill=MUTED)
    draw_terminal_lines(
        draw,
        x1 + 24,
        y1 + 82,
        [
            ("$ npm run dev", GOOD),
            ("ready in 412ms", TEXT),
            ("watching src/, shared/, packages/ui", MUTED),
        ],
    )


def clearer_progressive_chrome() -> Image.Image:
    img = app_shell()
    draw = ImageDraw.Draw(img)
    draw_root_tabs(draw)
    draw_sidebar(draw)
    draw.text((26, 84), "active workspace", font=F10, fill=MUTED)

    # active nested tabs only on top editor split
    round_rect(draw, (14, 76, 454, 108), (25, 31, 41, 255), outline=(255, 255, 255, 14), radius=12)
    x = 30
    x = tab(draw, x, 82, "freeflow-web", active=True, compact=True)
    x = tab(draw, x, 82, "freeflow-ai", active=False, compact=True)
    chip(draw, W - 190, 82, "active split", tone="soft", compact=True)

    draw_editor_surface(draw, (210, 110, W - 18, 520), "freeflow-web")
    draw.text((W - 360, 140), "Only this split keeps full tab + panel chrome", font=F12, fill=MUTED)
    draw.text((W - 240, 170), "search   git   split   ⋯", font=F12, fill=MUTED)

    # bottom uses one shared strip, children panels are quieter
    round_rect(draw, (14, 536, W - 18, 568), (25, 31, 41, 255), outline=(255, 255, 255, 14), radius=12)
    x = 30
    x = tab(draw, x, 542, "web-server-freeflow", active=False, compact=True)
    x = tab(draw, x, 542, "claude-freeflow-web", active=False, compact=True)
    tab(draw, x, 542, "markdown", active=True, compact=True)
    chip(draw, W - 100, 542, "tools", tone="soft", compact=True)
    draw_terminal_panel(draw, (14, 572, 950, H - 44), compact=True)
    draw_markdown_panel(draw, (956, 572, W - 18, H - 44), unified=False)

    add_top_badge(img, "Progressive Chrome", "Nested splits stay, but only the focused level gets strong chrome.")
    add_footer_note(img, "Here: top editor is primary; bottom terminal/markdown area remains available but visually quieter.")
    return img


def clearer_single_surface() -> Image.Image:
    img = app_shell()
    draw = ImageDraw.Draw(img)
    draw_root_tabs(draw, style="minimal")
    draw_sidebar(draw, modern=True)

    # one continuous material
    round_rect(draw, (8, 78, W - 10, H - 36), (23, 29, 38, 255), outline=(255, 255, 255, 14), radius=18)
    line(draw, (202, 90, 202, H - 50), (255, 255, 255, 16), 1)
    line(draw, (10, 534, W - 12, 534), (255, 255, 255, 16), 1)
    line(draw, (956, 566, 956, H - 48), (255, 255, 255, 16), 1)
    draw.text((220, 92), "Everything lives on one surface: headers, tabs and content stop looking like stacked boxes.", font=F14, fill=TEXT)

    # tabs are text + underline
    draw.text((226, 122), "freeflow-web", font=F12, fill=TEXT)
    draw.text((344, 122), "freeflow-ai", font=F12, fill=MUTED)
    line(draw, (226, 142, 314, 142), ACCENT, 3)
    draw_editor_surface(draw, (220, 156, W - 18, 520), "freeflow-web", modern=True, unified=True)

    draw.text((30, 122), "Files", font=F12, fill=TEXT)
    draw.text((76, 122), "Search", font=F12, fill=MUTED)
    draw.text((132, 122), "Git", font=F12, fill=MUTED)

    # bottom split still exists, but it is integrated
    draw.text((30, 548), "web-server-freeflow", font=F12, fill=MUTED)
    draw.text((212, 548), "claude-freeflow-web", font=F12, fill=MUTED)
    draw.text((424, 548), "markdown", font=F12, fill=TEXT)
    line(draw, (424, 566, 504, 566), ACCENT, 3)
    draw_terminal_panel(draw, (14, 574, 950, H - 44), compact=True)
    draw_markdown_panel(draw, (956, 574, W - 18, H - 44), modern=True, unified=True)

    add_top_badge(img, "Single-Surface Minimal", "Same layout, but all chrome is visually flattened into one material.")
    add_footer_note(img, "Here: no heavy panel boxes. Hierarchy comes from text, underline, spacing and active color.")
    return img


def clearer_dock_first() -> Image.Image:
    img = app_shell()
    draw = ImageDraw.Draw(img)
    draw_root_tabs(draw)
    draw_sidebar(draw)

    # main canvas dedicated to editor
    round_rect(draw, (14, 76, 420, 108), (25, 31, 41, 255), outline=(255, 255, 255, 14), radius=12)
    x = 30
    x = tab(draw, x, 82, "freeflow-web", active=True, compact=True)
    x = tab(draw, x, 82, "freeflow-ai", active=False, compact=True)
    x = tab(draw, x, 82, "deploy", active=False, compact=True)
    chip(draw, W - 176, 82, "editor focus", tone="soft", compact=True)

    draw_editor_surface(draw, (210, 110, W - 18, 710), "freeflow-web")
    draw.text((W - 320, 140), "Center reserved for documents/editors", font=F12, fill=MUTED)

    # full width dock replaces bottom split
    round_rect(draw, (18, 736, W - 18, H - 36), (22, 28, 37, 255), outline=(255, 255, 255, 16), radius=18)
    round_rect(draw, (34, 752, 640, 786), SURFACE, radius=14)
    x = 48
    x = tab(draw, x, 758, "Terminal", active=True, compact=True)
    x = tab(draw, x, 758, "Markdown Preview", active=False, compact=True)
    x = tab(draw, x, 758, "Git", active=False, compact=True)
    x = tab(draw, x, 758, "Problems", active=False, compact=True)
    chip(draw, W - 112, 758, "dock", tone="warn", compact=True)
    round_rect(draw, (34, 796, W - 34, H - 52), (12, 16, 22, 255), radius=14)
    draw_terminal_lines(
        draw,
        58,
        816,
        [
            ("$ npm run dev", GOOD),
            ("ready in 410ms   local: http://localhost:5173", TEXT),
            ("watching src/, shared/, packages/ui", MUTED),
            ("$ cargo test -p pax-gui", GOOD),
            ("running 94 tests   94 passed", TEXT),
        ],
    )
    draw.text((W - 430, 816), "Markdown preview, Git and Search stop fragmenting the main canvas.", font=F14, fill=TEXT)

    add_top_badge(img, "Dock-First Workspace", "Bottom tools become a persistent dock; center remains editor-first.")
    add_footer_note(img, "Here: the current bottom terminal + markdown split is replaced by one dock with multiple tool tabs.")
    return img


def progressive_chrome(base: Image.Image) -> Image.Image:
    img = base.copy().convert("RGBA")
    img = soften_area(img, (0, 28, W, 108), (18, 23, 30, 210))
    img = soften_area(img, (0, 408, W, 470), (18, 23, 30, 200))
    draw = ImageDraw.Draw(img)

    # Root tabs: slim and detached from child tabs.
    round_rect(draw, (14, 38, 330, 70), SURFACE_3, outline=(255, 255, 255, 16), radius=13)
    x = 30
    for label, active in [("freeflow", True), ("deploy", False)]:
        x = tab(draw, x, 44, label, active=active, compact=True)

    # Only the active nested split gets a real tab strip.
    round_rect(draw, (12, 74, 468, 102), (22, 28, 37, 220), radius=12)
    x = 24
    for label, active in [("freeflow-web", True), ("freeflow-ai", False)]:
        x = tab(draw, x, 76, label, active=active, compact=True)
    x = chip(draw, x + 12, 78, "code editor", tone="active", compact=True)
    chip(draw, W - 284, 78, "active split", tone="soft", compact=True)

    # Active top panel gets one compact chrome line, not layered bars.
    panel_surface(
        draw,
        (204, 110, W - 16, 404),
        title="freeflow-web",
        subtitle="editor header merged with tabs, actions pushed to the right",
        active=True,
        radius=14,
    )
    top_canvas_hint(draw, 1210, 138, "inactive nested headers collapse into faint labels")
    line(draw, (222, 154, W - 34, 154), (255, 255, 255, 14), 1)
    x = 228
    x = chip(draw, x, 124, "README.md", tone="muted")
    x = chip(draw, x, 124, "app.rs", tone="muted")
    chip(draw, x, 124, "workspaces.json", tone="active")
    draw.text((W - 210, 126), "search   git   split   ⋯", font=F12, fill=MUTED)
    draw_code_lines(draw, 248, 180, 760, rows=9)

    # Bottom split uses a single shared strip; child headers become tiny.
    round_rect(draw, (12, 418, W - 16, 450), SURFACE_3, outline=(255, 255, 255, 14), radius=12)
    x = 30
    for label, active in [("webserver-freeflow", False), ("claude-freeflow-web", False), ("markdown", True)]:
        x = tab(draw, x, 428, label, active=active, compact=True)
    chip(draw, W - 170, 425, "tools", tone="soft", compact=True)

    panel_surface(draw, (10, 452, 936, H - 48), title="terminal", subtitle="inactive chrome reduced to label only", radius=12)
    panel_surface(draw, (940, 452, W - 16, H - 48), title="markdown", subtitle="active panel keeps compact toolbar", active=True, radius=12)
    line(draw, (940, 452, 940, H - 48), (255, 255, 255, 18), 1)
    draw.text((972, 480), "edit   preview   refresh", font=F12, fill=MUTED)
    draw_terminal_lines(
        draw,
        30,
        500,
        [
            ("$ npm run dev", GOOD),
            ("ready in 406ms", TEXT),
            ("watching src/ and packages/ui", MUTED),
        ],
    )
    draw.text((968, 520), "Live markdown preview / notes panel", font=F14, fill=TEXT)

    add_top_badge(img, "Progressive Chrome", "Only the focused split keeps strong tabs and actions.")
    add_footer_note(img, "Concrete effect: your current nested split page keeps the same layout, but only one level at a time looks 'strong'.")
    return img


def single_surface_minimal(base: Image.Image) -> Image.Image:
    img = base.copy().convert("RGBA")
    img = soften_area(img, (0, 28, W, H - 18), (17, 22, 29, 120))
    draw = ImageDraw.Draw(img)

    # The whole work area feels like one continuous plane.
    round_rect(draw, (8, 96, W - 10, H - 44), (18, 23, 30, 150), outline=(255, 255, 255, 14), radius=18)
    round_rect(draw, (12, 96, 198, H - 46), (23, 29, 38, 205), radius=14)
    line(draw, (199, 104, 199, H - 50), (255, 255, 255, 18), 1)
    line(draw, (12, 410, W - 12, 410), (255, 255, 255, 16), 1)
    line(draw, (940, 456, 940, H - 48), (255, 255, 255, 16), 1)

    # Top tabs become text-first with underline, not boxes.
    round_rect(draw, (14, 36, 430, 70), (20, 26, 34, 195), radius=14)
    draw.text((32, 45), "freeflow", font=F14, fill=TEXT)
    draw.text((136, 45), "deploy", font=F14, fill=MUTED)
    draw.text((220, 45), "review", font=F14, fill=MUTED)
    line(draw, (30, 66, 92, 66), ACCENT, 3)

    # Sidebar and toolbars disappear into the same material.
    round_rect(draw, (18, 108, 188, 136), (25, 31, 40, 220), radius=10)
    draw.text((30, 116), "Files   Search   Git", font=F12, fill=MUTED)
    draw.text((220, 116), "Everything shares one surface. Visual hierarchy comes from text, spacing and underline.", font=F14, fill=(210, 218, 228, 235))
    draw_code_lines(draw, 230, 168, 840, rows=10)

    # Bottom split becomes visually lighter, still present.
    round_rect(draw, (16, 420, W - 14, 444), (22, 28, 37, 175), radius=12)
    draw.text((36, 427), "webserver-freeflow", font=F12, fill=MUTED)
    draw.text((214, 427), "claude-freeflow-web", font=F12, fill=MUTED)
    draw.text((428, 427), "markdown", font=F12, fill=TEXT)
    line(draw, (427, 444, 505, 444), ACCENT, 3)

    # Panel chrome nearly disappears.
    draw.text((28, 460), "terminal", font=F10, fill=MUTED)
    draw.text((958, 460), "markdown", font=F10, fill=MUTED)
    draw.text((972, 480), "preview refresh", font=F12, fill=MUTED)
    draw_terminal_lines(
        draw,
        30,
        500,
        [
            ("$ npm run dev", GOOD),
            ("ready in 410ms", TEXT),
        ],
    )
    draw.text((972, 520), "Preview content becomes part of the same material", font=F14, fill=TEXT)
    top_canvas_hint(draw, 1110, 430, "panels still split, but surfaces no longer stack visually")

    add_top_badge(img, "Single-Surface Minimal", "One continuous workspace surface with lighter separators and softer tabs.")
    add_footer_note(img, "Concrete effect: same page, but toolbar/sidebar/panel boundaries stop competing with the actual content.")
    return img


def dock_first(base: Image.Image) -> Image.Image:
    img = base.copy().convert("RGBA")
    draw = ImageDraw.Draw(img)

    # Top becomes editor-first.
    img = soften_area(img, (0, 28, W, 610), (18, 22, 30, 120))
    round_rect(draw, (14, 38, 420, 72), SURFACE_3, outline=(255, 255, 255, 14), radius=14)
    x = 30
    for label, active in [("freeflow-web", True), ("freeflow-ai", False), ("deploy", False)]:
        x = tab(draw, x, 44, label, active=active, compact=True)
    draw.text((W - 356, 48), "Main area reserved for editors", font=F16, fill=(208, 217, 227, 235))

    # Remove the current bottom split and replace with a full-width dock.
    dock_y = 690
    round_rect(draw, (12, 420, W - 12, H - 46), (14, 18, 24, 236), radius=16)
    panel_surface(draw, (16, 94, W - 16, 674), title="freeflow-web", subtitle="code editor remains the dominant canvas", active=True, radius=14)
    x = 226
    x = chip(draw, x, 120, "src/main.ts", tone="active")
    x = chip(draw, x, 120, "README.md", tone="muted")
    x = chip(draw, x, 120, "settings.json", tone="muted")
    chip(draw, W - 270, 120, "editor focus", tone="soft")
    draw_code_lines(draw, 252, 174, 980, rows=16)
    top_canvas_hint(draw, 1228, 154, "nested bottom panels removed from center")

    round_rect(draw, (22, dock_y, W - 22, H - 48), (19, 25, 33, 244), outline=(255, 255, 255, 18), radius=18)
    round_rect(draw, (34, dock_y + 16, 620, dock_y + 48), SURFACE, radius=14)
    x = 48
    for label, active in [("Terminal", True), ("Markdown Preview", False), ("Git", False), ("Problems", False)]:
        x = tab(draw, x, dock_y + 22, label, active=active, compact=True)
    chip(draw, W - 188, dock_y + 20, "dock", tone="warn", compact=True)
    draw.text((W - 430, dock_y + 26), "Persistent tool area instead of nested bottom splits", font=F14, fill=MUTED)

    # Paint the dock content concretely as terminal-first.
    round_rect(draw, (34, dock_y + 66, W - 34, H - 66), (12, 16, 22, 245), radius=14)
    draw_terminal_lines(
        draw,
        54,
        dock_y + 84,
        [
            ("$ npm run dev", GOOD),
            ("ready in 410ms   local: http://localhost:5173", TEXT),
            ("watching src/, shared/, packages/ui", MUTED),
            ("$ cargo test -p pax-gui", GOOD),
            ("running 94 tests   94 passed", TEXT),
            ("markdown preview available in dock tab", MUTED),
        ],
    )
    top_canvas_hint(draw, 1124, dock_y + 22, "markdown/git/search move here when needed")

    add_top_badge(img, "Dock-First Workspace", "Keep editors in the center; push tools into a persistent bottom dock.")
    add_footer_note(img, "Concrete effect: your current bottom terminal + markdown split becomes one dock, so the center stops fragmenting.")
    return img


def contact_sheet(images: list[tuple[str, Image.Image]]) -> Image.Image:
    margin = 28
    title_h = 84
    thumb_w = 920
    thumb_h = int(thumb_w * H / W)
    canvas = Image.new("RGBA", (margin * 3 + thumb_w * 2, title_h + margin * 3 + thumb_h * 2), BG)
    draw = ImageDraw.Draw(canvas)
    draw.text((margin, 24), "Pax UI Direction Previews", font=F24B, fill=TEXT)
    draw.text((margin, 52), "Based on current graphite screenshot. These are visual direction mockups, not final pixel-perfect designs.", font=F12, fill=MUTED)

    positions = [
        (margin, title_h + margin),
        (margin * 2 + thumb_w, title_h + margin),
        (margin, title_h + margin * 2 + thumb_h),
        (margin * 2 + thumb_w, title_h + margin * 2 + thumb_h),
    ]
    for (label, image), (x, y) in zip(images, positions):
        thumb = image.resize((thumb_w, thumb_h), Image.Resampling.LANCZOS)
        round_rect(draw, (x - 1, y - 1, x + thumb_w + 1, y + thumb_h + 1), (255, 255, 255, 0), outline=(255, 255, 255, 20), radius=18)
        canvas.alpha_composite(thumb, (x, y))
        round_rect(draw, (x + 18, y + 16, x + 280, y + 48), (10, 14, 20, 196), radius=12)
        draw.text((x + 32, y + 24), label, font=F18B, fill=TEXT)
    return canvas


def main():
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    base = Image.open(BASE_IMAGE).convert("RGBA")

    images = [
        ("Original", base),
        ("Progressive Chrome", progressive_chrome(base)),
        ("Single-Surface Minimal", single_surface_minimal(base)),
        ("Dock-First", dock_first(base)),
    ]
    for label, image in images[1:]:
        path = OUT_DIR / f"{label.lower().replace(' ', '-').replace('/', '-')}.png"
        image.save(path)

    sheet = contact_sheet(images)
    sheet.save(OUT_DIR / "ui-directions-contact-sheet.png")

    clearer = [
        ("Progressive Chrome UI", clearer_progressive_chrome()),
        ("Single-Surface UI", clearer_single_surface()),
        ("Dock-First UI", clearer_dock_first()),
    ]
    for label, image in clearer:
        path = OUT_DIR / f"{label.lower().replace(' ', '-').replace('/', '-')}.png"
        image.save(path)


if __name__ == "__main__":
    main()
