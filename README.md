# Loopback Router

**Autore:** Roany Romano · **Licenza:** [Apache 2.0](LICENSE)

Router audio tra applicazioni per **macOS** e **Windows**: prende l'audio da qualsiasi
ingresso (microfono, interfaccia audio, o l'uscita di un'altra app) e lo manda a qualsiasi
uscita, con più rotte contemporanee mixate tra loro, gain e mute in tempo reale, meter e
diagnostica di latenza.

- **Motore in Rust** (`cpal`): CoreAudio su macOS, WASAPI su Windows. Callback audio senza
  lock né allocazioni, ring buffer SPSC, conversione di sample rate con interpolazione cubica
  e compensazione del drift di clock tra dispositivi diversi.
- **Interfaccia in Python** (tkinter, incluso in Python: nessuna dipendenza esterna),
  collegata al motore con PyO3. Preset in JSON, sessione ripristinata all'avvio.

## Come funziona il "loopback" tra app

Le app non possono scambiarsi audio direttamente: serve un **dispositivo audio virtuale**,
cioè un "cavo" che il sistema vede come scheda audio. Quello che un'app manda alla sua
uscita ricompare come ingresso. Loopback Router usa i driver virtuali gratuiti standard e
fa il lavoro di instradamento, mix e monitoraggio:

| Sistema | Driver virtuale | Nome del dispositivo |
|---|---|---|
| macOS | [BlackHole](https://existential.audio/blackhole/) (2ch o 16ch) | `BlackHole 2ch` |
| Windows | [VB-CABLE](https://vb-audio.com/Cable/) | `CABLE Input` (uscita) / `CABLE Output` (ingresso) |

Se nessun driver virtuale è installato, l'app lo segnala con un banner e un link. I
dispositivi virtuali compaiono in cima alle liste.

```
 Spotify ──► [BlackHole 2ch] ──┐
                               ├─► Loopback Router ──► [BlackHole 16ch] ──► OBS / Zoom
 Microfono / interfaccia ──────┘          │
                                          └──────────► Cuffie (monitor)
```

## Esempi

**Microfono + musica di un'app in una call o in OBS (macOS)**
1. Installa BlackHole 2ch e BlackHole 16ch (o usa canali diversi di uno solo).
2. In Spotify (o nelle Impostazioni di sistema) imposta l'uscita su `BlackHole 2ch`.
3. Rotte: `Microfono → BlackHole 16ch` e `BlackHole 2ch → BlackHole 16ch` (gain −12 dB).
4. In Zoom/OBS scegli `BlackHole 16ch` come microfono.
5. Opzionale: `BlackHole 2ch → Cuffie` per sentire la musica anche tu.

**Chitarra dall'interfaccia audio verso un'app (Windows)**
1. Rotta `Interfaccia (canale 1 mono) → CABLE Input`.
2. Nell'app di destinazione scegli `CABLE Output` come ingresso.

Il menu dei canali permette di scegliere coppie stereo (`1+2`, `3+4`…) o un singolo canale
(`1 mono`, utile per un ingresso strumento); su una destinazione mono L e R vengono mediati.

## Uso

- **▶ Avvia / ■ Stop** apre e chiude i dispositivi.
- **Gain** (−∞…+12 dB, doppio clic sul valore per tornare a 0 dB) e **Mute** si applicano
  dal vivo, senza interruzioni. Cambiare dispositivo o canali riapre gli stream (pausa di un
  istante).
- Sotto ogni rotta: meter di picco (verde < −12, giallo < −3, rosso), latenza effettiva del
  buffer, correzione del drift in ppm e conteggio degli xrun (underrun/overrun).
- **Latenza** è il valore minimo desiderato. Se il driver consegna blocchi più grandi,
  il buffer si adatta da solo per evitare dropout.
- **Salva… / Carica…**: preset JSON. L'ultima sessione è salvata automaticamente.
- Una rotta da un dispositivo virtuale a sé stesso crea feedback: l'app chiede conferma.

Da riga di comando:

```bash
loopback --list                               # elenca dispositivi, canali, sample rate
loopback --preset streaming.json --headless   # routing senza interfaccia (Ctrl+C per uscire)
```

## Compilare

Prerequisiti: Python 3.9+ da python.org (include Tk) e Rust da <https://rustup.rs>
(su Windows con la toolchain MSVC / Visual Studio Build Tools).

```bash
# macOS → dist/Loopback Router.app
bash scripts/build_mac.sh

# Windows (PowerShell) → dist\Loopback Router\Loopback Router.exe
pwsh scripts/build_windows.ps1
```

Gli script eseguono i test, compilano il motore con `maturin` e impacchettano con
PyInstaller. Per sviluppo:

```bash
python -m venv .venv && source .venv/bin/activate
pip install maturin pytest
maturin develop --release
python -m loopback
```

Con un repository GitHub, il workflow in `.github/workflows/build.yml` compila e pubblica
gli zip per macOS (Apple Silicon) e Windows a ogni tag `v*`.

**macOS**: al primo avvio il sistema chiede il permesso per il microfono (necessario per
leggere qualsiasi ingresso, anche BlackHole). L'app non è firmata con un certificato
Apple Developer: la prima volta aprila con clic destro → Apri. Se l'avevi negato:
Impostazioni di sistema → Privacy e sicurezza → Microfono.

## Test

| Livello | Cosa verifica |
|---|---|
| `cargo test --no-default-features` | 15 test DSP: conversione 44,1→48 kHz (frequenza e THD+N < −80 dB), controller del drift con un clock sfasato di 300 ppm per 60 s, priming/underrun, latenza adattiva, gestione rotte |
| `pytest tests` | preset, controller (gain/mute dal vivo senza riavvio, riavvio sui cambi strutturali), binding Rust |
| `tests/integration/pulse_loopback.sh` | Linux, audio reale: sinusoide a 44,1 kHz in un sink virtuale → router → secondo sink a 48 kHz → registrazione analizzata campione per campione |

Risultati del test di integrazione in sviluppo (PulseAudio, sink a 44,1 e 48 kHz,
10 s di registrazione per run): 0 glitch e rumore di fondo −86 dB rispetto al segnale (al
limite dei 16 bit della registrazione) in 7 run su 7 con latenza richiesta 5, 10 e 20 ms. Con
i blocchi grandi di PulseAudio la latenza effettiva si assesta intorno a 30 ms; su CoreAudio
e WASAPI i blocchi sono più piccoli e ci si aspetta meno, ma **su Mac e Windows reali
non è ancora stato misurato**: i valori lì vanno verificati.

## Limiti attuali

- Non crea un proprio driver virtuale: usa BlackHole / VB-CABLE (scrivere un driver
  richiede un AudioServerPlugIn firmato su macOS e un driver kernel firmato su Windows).
- Non cattura il suono di una singola app: si imposta l'uscita di quell'app sul
  dispositivo virtuale.
- Windows: modalità WASAPI condivisa (ASIO non abilitato).
- Usa il formato di default di ogni dispositivo; il sample rate si cambia nelle
  impostazioni audio del sistema (su Mac: Configurazione Audio MIDI).

## Struttura

```
src/dsp.rs        resampler cubico con controllo del drift, controlli lock-free, soft clip
src/engine.rs     dispositivi, stream cpal, ring buffer, mixer per uscita
src/lib.rs        binding PyO3 → modulo loopback._engine
python/loopback/  controller, preset, interfaccia tkinter, CLI
packaging/        spec PyInstaller (Info.plist, proprietà .exe Windows)
scripts/          build macOS / Windows
tests/            test Python e test d'integrazione audio
```

## Licenza

Copyright 2026 Roany Romano.

Distribuito con licenza [Apache 2.0](LICENSE): chiunque può usare, modificare e
ridistribuire il software, anche in prodotti commerciali, a condizione di includere la
licenza e il file [NOTICE](NOTICE) con l'attribuzione all'autore e di indicare i file
modificati. Le dipendenze di terze parti e le loro licenze sono elencate in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
