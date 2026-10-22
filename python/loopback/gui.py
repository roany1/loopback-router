# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 Roany Romano

"""Tkinter user interface (bundled with Python on macOS and Windows: no extra deps)."""

from __future__ import annotations

import sys
import tkinter as tk
import webbrowser
from pathlib import Path
from tkinter import filedialog, messagebox, ttk

from . import __copyright__, __license__, __version__
from .controller import RouterController
from .presets import (
    Preset,
    RouteConfig,
    channel_label,
    channel_options,
    last_session_path,
    presets_dir,
)

# ---------------------------------------------------------------- palette
BG = "#16181d"
PANEL = "#1f2229"
PANEL_HI = "#272b34"
FG = "#e6e8ee"
MUTED = "#8a90a0"
ACCENT = "#4fd1a5"
WARN = "#f2c14e"
DANGER = "#ef5d60"
METER_BG = "#0f1115"

IS_MAC = sys.platform == "darwin"
IS_WIN = sys.platform.startswith("win")
UI_FONT = ("SF Pro Text", 12) if IS_MAC else ("Segoe UI", 10) if IS_WIN else ("DejaVu Sans", 10)
UI_BOLD = (UI_FONT[0], UI_FONT[1], "bold")
TITLE_FONT = (UI_FONT[0], UI_FONT[1] + 6, "bold")
MONO = ("Menlo", 11) if IS_MAC else ("Consolas", 9) if IS_WIN else ("DejaVu Sans Mono", 9)

VIRTUAL_DRIVER = (
    ("BlackHole", "https://existential.audio/blackhole/")
    if IS_MAC
    else ("VB-CABLE", "https://vb-audio.com/Cable/")
)

LATENCIES = ["5", "10", "20", "40", "80"]
POLL_MS = 50
METER_W, METER_H = 140, 10
METER_FLOOR = -60.0


def _setup_style(root: tk.Tk) -> None:
    root.configure(bg=BG)
    s = ttk.Style(root)
    s.theme_use("clam")
    s.configure(".", background=BG, foreground=FG, font=UI_FONT, fieldbackground=PANEL_HI)
    s.configure("TFrame", background=BG)
    s.configure("Panel.TFrame", background=PANEL)
    s.configure("TLabel", background=BG, foreground=FG)
    s.configure("Panel.TLabel", background=PANEL, foreground=FG)
    s.configure("Muted.TLabel", background=PANEL, foreground=MUTED, font=MONO)
    s.configure("Title.TLabel", background=BG, foreground=FG, font=TITLE_FONT)
    s.configure("Sub.TLabel", background=BG, foreground=MUTED)
    s.configure("Arrow.TLabel", background=PANEL, foreground=ACCENT, font=TITLE_FONT)
    s.configure("Banner.TFrame", background="#3a3220")
    s.configure("Banner.TLabel", background="#3a3220", foreground=WARN)
    s.configure("TButton", background=PANEL_HI, foreground=FG, borderwidth=0, padding=(10, 5))
    s.map("TButton", background=[("active", "#323744")])
    s.configure("Start.TButton", background=ACCENT, foreground="#0b1a14", font=UI_BOLD, padding=(16, 6))
    s.map("Start.TButton", background=[("active", "#6be0b8")])
    s.configure("Stop.TButton", background=DANGER, foreground="#22090a", font=UI_BOLD, padding=(16, 6))
    s.map("Stop.TButton", background=[("active", "#f47c7e")])
    s.configure("Icon.TButton", background=PANEL, foreground=MUTED, padding=(6, 2))
    s.map("Icon.TButton", background=[("active", PANEL_HI)], foreground=[("active", DANGER)])
    flat = dict(bordercolor=PANEL_HI, lightcolor=PANEL_HI, darkcolor=PANEL_HI)
    s.configure("TCombobox", fieldbackground=PANEL_HI, background=PANEL_HI, foreground=FG,
                arrowcolor=MUTED, borderwidth=1, padding=(6, 3), **flat)
    s.map("TCombobox", fieldbackground=[("readonly", PANEL_HI)], foreground=[("readonly", FG)],
          selectbackground=[("readonly", PANEL_HI)], selectforeground=[("readonly", FG)],
          bordercolor=[("focus", ACCENT)], arrowcolor=[("active", FG)])
    root.option_add("*TCombobox*Listbox.background", PANEL_HI)
    root.option_add("*TCombobox*Listbox.foreground", FG)
    root.option_add("*TCombobox*Listbox.selectBackground", ACCENT)
    s.configure("TCheckbutton", background=PANEL, foreground=FG)
    s.map("TCheckbutton", background=[("active", PANEL)])
    s.configure("TCheckbutton", indicatorbackground=PANEL_HI, indicatorforeground=DANGER,
                indicatorrelief="flat", focuscolor=PANEL)
    s.map("TCheckbutton", indicatorbackground=[("selected", PANEL_HI)])
    s.configure("Horizontal.TScale", background=MUTED, troughcolor=PANEL_HI, sliderthickness=12,
                **flat)
    s.map("Horizontal.TScale", background=[("active", FG)])
    s.configure("Vertical.TScrollbar", background=PANEL_HI, troughcolor=BG, arrowcolor=MUTED,
                bordercolor=BG, lightcolor=PANEL_HI, darkcolor=PANEL_HI)


class Meter(tk.Canvas):
    """Peak meter with fall-off, -60..0 dBFS, green/yellow/red zones."""

    def __init__(self, master):
        super().__init__(master, width=METER_W, height=METER_H, bg=METER_BG,
                         highlightthickness=0, bd=0)
        self._shown = METER_FLOOR
        self._bar = self.create_rectangle(0, 0, 0, METER_H, fill=ACCENT, width=0)
        # 0 dB and -12 dB ticks
        for db in (-12.0, -6.0):
            x = self._x(db)
            self.create_line(x, 0, x, METER_H, fill="#2c303a")

    @staticmethod
    def _x(db: float) -> float:
        db = max(METER_FLOOR, min(0.0, db))
        return (db - METER_FLOOR) / -METER_FLOOR * METER_W

    def update_db(self, db: float) -> None:
        # Instant attack, ~20 dB/s release at 50 ms polling
        self._shown = max(db, self._shown - 1.0, METER_FLOOR)
        color = ACCENT if self._shown < -12 else WARN if self._shown < -3 else DANGER
        self.coords(self._bar, 0, 0, self._x(self._shown), METER_H)
        self.itemconfigure(self._bar, fill=color)

    def reset(self) -> None:
        self._shown = METER_FLOOR
        self.coords(self._bar, 0, 0, 0, METER_H)


class RouteRow(ttk.Frame):
    def __init__(self, master, app: "App", index: int):
        super().__init__(master, style="Panel.TFrame", padding=(12, 10))
        self.app, self.index = app, index
        rc = app.ctl.preset.routes[index]

        self.src_var = tk.StringVar(value=rc.source)
        self.dst_var = tk.StringVar(value=rc.dest)
        self.src_ch_var = tk.StringVar(value=channel_label(tuple(rc.src_channels)))
        self.dst_ch_var = tk.StringVar(value=channel_label(tuple(rc.dst_channels)))
        self.gain_var = tk.DoubleVar(value=rc.gain_db)
        self.mute_var = tk.BooleanVar(value=rc.mute)

        top = ttk.Frame(self, style="Panel.TFrame")
        top.pack(fill="x")
        self.src_cb = ttk.Combobox(top, textvariable=self.src_var, state="readonly", width=28,
                                   values=app.input_names)
        self.src_cb.pack(side="left")
        self.src_ch_cb = ttk.Combobox(top, textvariable=self.src_ch_var, state="readonly", width=7)
        self.src_ch_cb.pack(side="left", padx=(6, 0))
        ttk.Label(top, text="  →  ", style="Arrow.TLabel").pack(side="left")
        self.dst_cb = ttk.Combobox(top, textvariable=self.dst_var, state="readonly", width=28,
                                   values=app.output_names)
        self.dst_cb.pack(side="left")
        self.dst_ch_cb = ttk.Combobox(top, textvariable=self.dst_ch_var, state="readonly", width=7)
        self.dst_ch_cb.pack(side="left", padx=(6, 0))
        ttk.Button(top, text="✕", style="Icon.TButton", width=2,
                   command=self._remove).pack(side="right")

        bottom = ttk.Frame(self, style="Panel.TFrame")
        bottom.pack(fill="x", pady=(8, 0))
        ttk.Checkbutton(bottom, text="Mute", variable=self.mute_var,
                        command=self._on_mute).pack(side="left")
        ttk.Label(bottom, text="  Gain", style="Panel.TLabel").pack(side="left")
        ttk.Scale(bottom, from_=-60, to=12, variable=self.gain_var, length=160,
                  command=self._on_gain).pack(side="left", padx=6)
        self.gain_lbl = ttk.Label(bottom, width=8, style="Muted.TLabel")
        self.gain_lbl.pack(side="left")
        self.meter = Meter(bottom)
        self.meter.pack(side="left", padx=(8, 8))
        self.stats_lbl = ttk.Label(bottom, text="", style="Muted.TLabel")
        self.stats_lbl.pack(side="left")
        self.gain_lbl.bind("<Double-Button-1>", lambda e: self._reset_gain())

        self._refresh_channel_choices()
        self._show_gain()
        self.src_cb.bind("<<ComboboxSelected>>", lambda e: self._on_device(True))
        self.dst_cb.bind("<<ComboboxSelected>>", lambda e: self._on_device(False))
        self.src_ch_cb.bind("<<ComboboxSelected>>", lambda e: self._on_channels())
        self.dst_ch_cb.bind("<<ComboboxSelected>>", lambda e: self._on_channels())

    # ---- helpers
    def _refresh_channel_choices(self):
        dev = self.app.device_by_name
        n_in = dev[self.src_var.get()].input_channels if self.src_var.get() in dev else 2
        n_out = dev[self.dst_var.get()].output_channels if self.dst_var.get() in dev else 2
        self._src_opts = dict(channel_options(n_in))
        self._dst_opts = dict(channel_options(n_out))
        self.src_ch_cb["values"] = list(self._src_opts)
        self.dst_ch_cb["values"] = list(self._dst_opts)
        if self.src_ch_var.get() not in self._src_opts:
            self.src_ch_var.set(next(iter(self._src_opts)))
        if self.dst_ch_var.get() not in self._dst_opts:
            self.dst_ch_var.set(next(iter(self._dst_opts)))

    def _show_gain(self):
        g = self.gain_var.get()
        self.gain_lbl.configure(text="-∞ dB" if g <= -59.5 else f"{g:+.1f} dB")

    # ---- events
    def _on_device(self, is_src: bool):
        self._refresh_channel_choices()
        self._on_channels()

    def _on_channels(self):
        self.app.safe(lambda: self.app.ctl.update_route(
            self.index,
            source=self.src_var.get(),
            dest=self.dst_var.get(),
            src_channels=self._src_opts[self.src_ch_var.get()],
            dst_channels=self._dst_opts[self.dst_ch_var.get()],
        ))
        self.meter.reset()
        self.app.changed()

    def _on_gain(self, _=None):
        g = round(self.gain_var.get() * 2) / 2  # 0.5 dB steps
        self._show_gain()
        self.app.safe(lambda: self.app.ctl.update_route(self.index, gain_db=-100.0 if g <= -59.5 else g))
        self.app.changed()

    def _reset_gain(self):
        self.gain_var.set(0.0)
        self._on_gain()

    def _on_mute(self):
        self.app.safe(lambda: self.app.ctl.update_route(self.index, mute=self.mute_var.get()))
        self.app.changed()

    def _remove(self):
        self.app.safe(lambda: self.app.ctl.remove_route(self.index))
        self.app.changed()
        self.app.rebuild_rows()

    def update_status(self, st):
        if st is None:
            return
        self.meter.update_db(st.peak_db)
        xr = st.underruns + st.overruns
        txt = f"{st.latency_ms:5.1f} ms  {st.drift_ppm:+5d} ppm"
        if xr:
            txt += f"  xrun {xr}"
        self.stats_lbl.configure(text=txt, foreground=WARN if xr else MUTED)


class App:
    def __init__(self, root: tk.Tk):
        self.root = root
        root.title("Loopback Router")
        root.minsize(900, 420)
        _setup_style(root)

        preset = self._load_last_session()
        self.ctl = RouterController(preset)
        self._dirty = False
        self.rows: list[RouteRow] = []
        self.refresh_devices(initial=True)

        if not self.ctl.preset.routes:
            self._add_default_route()

        self._build_header()
        self._build_banner()
        self._build_footer()  # packed at the bottom first, so it is never pushed off-screen
        self._build_routes_area()
        self.rebuild_rows()
        self._update_run_button()

        root.protocol("WM_DELETE_WINDOW", self.on_close)
        if IS_MAC:
            try:  # standard "About Loopback Router" item in the app menu
                root.createcommand("tkAboutDialog", self.show_about)
            except tk.TclError:
                pass
        root.after(POLL_MS, self._poll)
        root.after(5000, self._autosave)

    # ------------------------------------------------------------ layout
    def _build_header(self):
        h = ttk.Frame(self.root, padding=(16, 14, 16, 6))
        h.pack(fill="x")
        left = ttk.Frame(h)
        left.pack(side="left")
        ttk.Label(left, text="Loopback Router", style="Title.TLabel").pack(anchor="w")
        self.sub_lbl = ttk.Label(left, style="Sub.TLabel",
                                 text=f"v{__version__} · driver {self.ctl.host_name()} · di Roany Romano")
        self.sub_lbl.pack(anchor="w")

        right = ttk.Frame(h)
        right.pack(side="right")
        self.run_btn = ttk.Button(right, command=self.toggle_run)
        self.run_btn.pack(side="right", padx=(10, 0))
        ttk.Label(right, text="Latenza (ms)").pack(side="left")
        self.lat_var = tk.StringVar(value=str(int(self.ctl.preset.latency_ms)))
        lat = ttk.Combobox(right, textvariable=self.lat_var, values=LATENCIES, width=4,
                           state="readonly")
        lat.pack(side="left", padx=(6, 10))
        lat.bind("<<ComboboxSelected>>",
                 lambda e: (self.safe(lambda: self.ctl.set_latency(float(self.lat_var.get()))),
                            self.changed()))
        ttk.Button(right, text="Carica…", command=self.load_preset).pack(side="left", padx=2)
        ttk.Button(right, text="Salva…", command=self.save_preset).pack(side="left", padx=2)
        ttk.Button(right, text="↻ Dispositivi", command=self.refresh_devices).pack(side="left", padx=2)

    def _build_banner(self):
        self.banner = ttk.Frame(self.root, style="Banner.TFrame", padding=(16, 8))
        name, url = VIRTUAL_DRIVER
        lbl = ttk.Label(
            self.banner, style="Banner.TLabel", cursor="hand2",
            wraplength=860, justify="left",
            text=(f"Nessun dispositivo virtuale trovato. Per instradare l'audio tra app installa "
                  f"{name} (gratuito, clicca qui per scaricarlo), poi premi ↻ Dispositivi."),
        )
        lbl.pack(anchor="w")
        lbl.bind("<Button-1>", lambda e: webbrowser.open(url))
        self._update_banner()

    def _build_routes_area(self):
        outer = ttk.Frame(self.root, padding=(16, 6))
        outer.pack(fill="both", expand=True)
        self.canvas = tk.Canvas(outer, bg=BG, highlightthickness=0, bd=0)
        sb = ttk.Scrollbar(outer, orient="vertical", command=self.canvas.yview)
        self.rows_frame = ttk.Frame(self.canvas)
        self.rows_frame.bind("<Configure>",
                             lambda e: self.canvas.configure(scrollregion=self.canvas.bbox("all")))
        win = self.canvas.create_window((0, 0), window=self.rows_frame, anchor="nw")
        self.canvas.bind("<Configure>", lambda e: self.canvas.itemconfigure(win, width=e.width))
        self.canvas.configure(yscrollcommand=sb.set)
        self.canvas.pack(side="left", fill="both", expand=True)
        sb.pack(side="right", fill="y")

    def _build_footer(self):
        f = ttk.Frame(self.root, padding=(16, 6, 16, 12))
        f.pack(side="bottom", fill="x")
        ttk.Button(f, text="＋ Aggiungi rotta", command=self.add_route).pack(side="left")
        ttk.Button(f, text="ⓘ Informazioni", command=self.show_about).pack(side="right")
        self.status_lbl = ttk.Label(f, text="", style="Sub.TLabel")
        self.status_lbl.pack(side="left", padx=12)

    # ------------------------------------------------------------ devices
    def refresh_devices(self, initial: bool = False):
        try:
            devs = self.ctl.devices()
        except Exception as e:  # noqa: BLE001
            devs = []
            if not initial:
                messagebox.showerror("Dispositivi", str(e))
        self.device_by_name = {d.name: d for d in devs}
        # Virtual devices first: they're what you route through most of the time.
        key = lambda d: (not d.is_virtual, d.name.lower())  # noqa: E731
        self.input_names = [d.name for d in sorted(devs, key=key) if d.is_input]
        self.output_names = [d.name for d in sorted(devs, key=key) if d.is_output]
        self.has_virtual = any(d.is_virtual for d in devs)
        if not initial:
            self._update_banner()
            self.rebuild_rows()
            self.set_status(f"{len(devs)} dispositivi trovati")

    def _update_banner(self):
        if self.has_virtual:
            self.banner.pack_forget()
        else:
            self.banner.pack(fill="x", after=self.root.winfo_children()[0])

    def _add_default_route(self):
        from . import _engine
        d_in, _ = _engine.default_devices()
        virt_out = next((n for n in self.output_names if self.device_by_name[n].is_virtual), None)
        # Some hosts report a generic alias ("Default Audio Device") as default:
        # only trust it if it's a real entry of the device list.
        if d_in not in self.input_names:
            d_in = next((n for n in self.input_names if not self.device_by_name[n].is_virtual), None)
        src = d_in or (self.input_names[0] if self.input_names else "")
        dst = virt_out or (self.output_names[0] if self.output_names else "")
        if src and dst:
            self.ctl.add_route(RouteConfig(source=src, dest=dst))

    # ------------------------------------------------------------ routes
    def rebuild_rows(self):
        for r in self.rows:
            r.destroy()
        self.rows = []
        for i in range(len(self.ctl.preset.routes)):
            row = RouteRow(self.rows_frame, self, i)
            row.pack(fill="x", pady=(0, 8))
            self.rows.append(row)
        if not self.rows:
            self.set_status("Nessuna rotta: premi “＋ Aggiungi rotta”.")

    def add_route(self):
        if not self.input_names or not self.output_names:
            messagebox.showwarning("Rotta", "Servono almeno un ingresso e un'uscita audio.")
            return
        prev = self.ctl.preset.routes[-1] if self.ctl.preset.routes else None
        src = prev.dest if prev and prev.dest in self.input_names else self.input_names[0]
        dst = next((n for n in self.output_names if n != src), self.output_names[0])
        self.safe(lambda: self.ctl.add_route(RouteConfig(source=src, dest=dst)))
        self.changed()
        self.rebuild_rows()

    # ------------------------------------------------------------ run
    def toggle_run(self):
        if self.ctl.running:
            self.ctl.stop()
            for r in self.rows:
                r.meter.reset()
            self.set_status("Fermato")
        else:
            loops = [r.source for r in self.ctl.preset.routes
                     if r.source == r.dest and not r.mute
                     and r.source in self.device_by_name and self.device_by_name[r.source].is_virtual]
            if loops and not messagebox.askyesno(
                    "Rischio feedback",
                    f"La rotta {loops[0]} → {loops[0]} rimanda l'uscita del dispositivo virtuale "
                    "nel suo stesso ingresso: si crea un anello di feedback (fischio, livello al "
                    "massimo).\n\nAvviare comunque?", icon="warning"):
                return
            if self.safe(self.ctl.start):
                self.set_status("In esecuzione")
        self._update_run_button()

    def _update_run_button(self):
        if self.ctl.running:
            self.run_btn.configure(text="■  Stop", style="Stop.TButton")
        else:
            self.run_btn.configure(text="▶  Avvia", style="Start.TButton")

    def _poll(self):
        if self.ctl.running:
            for row, st in zip(self.rows, self.ctl.status()):
                row.update_status(st)
            errs = self.ctl.take_errors()
            if errs:
                self.set_status(errs[-1], error=True)
        self.root.after(POLL_MS, self._poll)

    # ------------------------------------------------------------ presets
    def save_preset(self):
        presets_dir().mkdir(parents=True, exist_ok=True)
        p = filedialog.asksaveasfilename(
            initialdir=presets_dir(), defaultextension=".json",
            filetypes=[("Preset Loopback", "*.json")], title="Salva preset")
        if p:
            self.ctl.preset.name = Path(p).stem
            self.ctl.preset.save(Path(p))
            self.set_status(f"Preset salvato: {Path(p).name}")

    def load_preset(self):
        p = filedialog.askopenfilename(
            initialdir=presets_dir() if presets_dir().exists() else None,
            filetypes=[("Preset Loopback", "*.json")], title="Carica preset")
        if not p:
            return
        try:
            preset = Preset.load(Path(p))
        except Exception as e:  # noqa: BLE001
            messagebox.showerror("Preset", f"File non valido:\n{e}")
            return
        self.safe(lambda: self.ctl.load_preset(preset))
        self.lat_var.set(str(int(preset.latency_ms)))
        self.rebuild_rows()
        self._update_run_button()
        missing = {r.source for r in preset.routes if r.source not in self.input_names}
        missing |= {r.dest for r in preset.routes if r.dest not in self.output_names}
        if missing:
            self.set_status("Dispositivi non collegati: " + ", ".join(sorted(missing)), error=True)
        else:
            self.set_status(f"Preset caricato: {preset.name}")
        self.changed()

    def _load_last_session(self) -> Preset | None:
        try:
            return Preset.load(last_session_path())
        except Exception:  # noqa: BLE001
            return None

    def changed(self):
        self._dirty = True

    def _autosave(self):
        if self._dirty:
            try:
                self.ctl.preset.save(last_session_path())
                self._dirty = False
            except OSError:
                pass
        self.root.after(5000, self._autosave)

    # ------------------------------------------------------------ misc
    def safe(self, fn) -> bool:
        """Run an engine action, showing errors instead of crashing the UI."""
        try:
            fn()
            return True
        except Exception as e:  # noqa: BLE001
            self.set_status(str(e), error=True)
            self._update_run_button()
            return False
        finally:
            if hasattr(self, "run_btn"):
                self._update_run_button()

    def set_status(self, text: str, error: bool = False):
        if hasattr(self, "status_lbl"):
            self.status_lbl.configure(text=text, foreground=DANGER if error else MUTED)

    def show_about(self):
        messagebox.showinfo(
            "Informazioni",
            f"Loopback Router {__version__}\n\n{__copyright__}\n"
            f"Licenza {__license__}\n\n"
            "Motore audio in Rust (cpal), interfaccia in Python.\n"
            "Componenti di terze parti: vedi THIRD-PARTY-NOTICES.md.",
        )

    def on_close(self):
        try:
            self.ctl.preset.save(last_session_path())
        except OSError:
            pass
        self.ctl.stop()
        self.root.destroy()


def run() -> None:
    root = tk.Tk()
    App(root)
    root.mainloop()
