# Plan de implementación — `wallrs`
### Motor de live wallpapers para Wayland, en Rust, sobre wgpu/Vulkan

**Nombre del proyecto:** `wallrs` (placeholder — renombrar libremente antes de iniciar).
**Audiencia de este documento:** un agente de desarrollo (humano o LLM) que ejecutará el proyecto fase por fase.
**Cómo usar este documento:** cada fase de la sección 11 tiene entregables y criterios de aceptación verificables. No se debe avanzar a la fase N+1 sin cerrar los criterios de aceptación de la fase N. Los checkboxes se marcan a medida que se completan.

---

## 1. Alcance y no-objetivos

**En alcance (v1):**
- Un daemon (`wallrsd`) que renderiza wallpapers animados como fondo de escritorio en compositores Wayland con soporte de `wlr-layer-shell`.
- Tres tipos de contenido: imagen/capas con parallax, shaders estilo Shadertoy, video en loop.
- Multi-monitor, con asignación independiente de wallpaper por salida.
- Control en runtime vía un cliente CLI (`wallctl`) que habla con el daemon por socket, sin reiniciar el proceso.
- Audio reactivo capturado **exclusivamente vía PipeWire** (sin ruta de compatibilidad PulseAudio).

**Fuera de alcance (v1), explícitamente:**
- Compatibilidad con el formato/catálogo de Steam Workshop de Wallpaper Engine. No se parsea `.pkg`, no hay sistema de partículas, no hay motor de propiedades genérico. Si en el futuro se quiere importar contenido de ese ecosistema, será un conversor externo, fuera de este repositorio.
- X11. Solo Wayland.
- Wallpapers web (HTML/CSS/JS vía CEF o `wry`). Se documenta como extensión futura en la sección 14, pero no se diseña ni se implementa en v1.
- GNOME, por no implementar `wlr-layer-shell`. Soporte de KDE Plasma se trata como **no verificado** hasta la Fase 1 (ver sección 14).

---

## 2. Principios de arquitectura

1. **Núcleo agnóstico al contenido.** El motor (Wayland, wgpu, control, audio) no sabe qué es un "wallpaper de imagen" o "de shader" — solo conoce un trait `WallpaperRenderer`. Los tipos de contenido son crates independientes que implementan ese trait.
2. **Mínima superficie de FFI.** Cada dependencia en C (`libmpv`, `libpipewire`, Wayland) debe estar aislada en un único crate del workspace, nunca esparcida.
3. **Un solo event loop.** Todo lo que vive en el hilo principal (Wayland, render, socket de control) se dirige con `calloop`. No se mezcla con `tokio` ni otro runtime async — evita dos loops compitiendo por el mismo proceso.
4. **PipeWire, no PulseAudio.** El subsistema de audio asume PipeWire como servidor de sonido del sistema. No se implementa una ruta alterna de PulseAudio nativo (`libpulse`); si algún día se necesita correr en un sistema sin PipeWire, es una decisión explícita fuera de este plan.
5. **Configuración declarativa y simple**, no un lenguaje de propiedades genérico. Cada wallpaper es una carpeta con un manifiesto TOML legible a mano.
6. **Fallos aislados por output.** Un error renderizando el wallpaper de un monitor no debe tumbar el daemon completo ni afectar a los demás monitores.

---

## 3. Vista de alto nivel

```
┌─────────────────────────────── wallrsd (proceso único) ───────────────────────────────┐
│                                                                                         │
│   calloop event loop (hilo principal)                                                  │
│   ├─ Wayland queue (calloop-wayland-source) ─ eventos de compositor, outputs, frame cb  │
│   ├─ Socket de control (Generic FD source)   ─ comandos de wallctl                      │
│   ├─ Fullscreen detector (wlr-foreign-toplevel-management)                              │
│   └─ Por cada output: superficie wlr-layer-shell + wgpu::Surface + instancia de un      │
│      WallpaperRenderer (image | shader | video)                                        │
│                                                                                         │
│   hilo de audio (independiente) ── PipeWire MainLoop propio ── FFT (rustfft) ──►        │
│      SpectrumHandle (lock-free, leído por el hilo principal en cada frame)              │
│                                                                                         │
└─────────────────────────────────────────────────────────────────────────────────────────┘
                       ▲
                       │ Unix domain socket ($XDG_RUNTIME_DIR/wallrs.sock)
                       │
                 wallctl (binario CLI, proceso separado, sin estado)
```

Dos hilos en total: el hilo principal (Wayland + render + control) y el hilo de audio (PipeWire tiene su propio mainloop en C, no se puede compartir trivialmente con `calloop`, así que corre aislado y entrega datos por un canal sin bloqueo).

---

## 4. Estructura del workspace

```
wallrs/
├── Cargo.toml                  # workspace
├── crates/
│   ├── wallrs-proto/           # tipos de mensajes IPC (serde), sin lógica
│   ├── wallrs-render/          # trait WallpaperRenderer, FrameContext, helpers wgpu comunes
│   ├── wallrs-audio/           # captura PipeWire + FFT, expone SpectrumHandle
│   ├── wallrs-content-image/   # content-type: imagen/capas + parallax
│   ├── wallrs-content-shader/  # content-type: shader estilo Shadertoy
│   ├── wallrs-content-video/   # content-type: video vía libmpv2 (render por software)
│   ├── wallrs-core/            # Wayland/layer-shell, gestión de outputs, calloop, socket
│   ├── wallrs-daemon/          # binario `wallrsd`: ensambla core + audio + content-types
│   └── wallrs-cli/             # binario `wallctl`: cliente de control
```

**Regla de dependencias:** `wallrs-content-*` solo depende de `wallrs-render` (que a su vez solo depende de `wgpu`). Nunca dependen de `wallrs-core` ni de Wayland directamente. Esto es lo que permite que mañana se agregue un content-type nuevo (o incluso se reuse `wallrs-render` en un proyecto distinto, p. ej. un salvapantallas) sin tocar el núcleo.

---

## 5. Pila tecnológica y justificación

| Componente           | Crate(s)                                                                        | Por qué                                                                                                                                                                                                       |
| -------------------- | ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Cliente Wayland      | `smithay-client-toolkit` (0.21.x, features `calloop`) + `wayland-protocols-wlr` | Toolkit estándar de facto para clientes Wayland en Rust; ya trae binding de `wlr-layer-shell` y `xdg-output`, y sus propios dev-dependencies confirman el patrón wgpu + `raw-window-handle` que vamos a usar. |
| Event loop           | `calloop` + `calloop-wayland-source`                                            | Integra la cola de eventos Wayland, el socket de control y timers en un único loop síncrono, sin runtime async.                                                                                               |
| Render               | `wgpu` (backend Vulkan en Linux) + `naga` (incluido)                            | Abstracción seguro sobre Vulkan; `naga` traduce/valida shaders sin depender de glslang/SPIRV-Cross externos.                                                                                                  |
| Puente ventana↔GPU   | `raw-window-handle`                                                             | Estándar de facto para pasar un `wl_surface` de SCTK a `wgpu::Surface`.                                                                                                                                       |
| Imágenes             | `image`                                                                         | Decodificación PNG/JPEG/WebP 100% Rust, sin FFI.                                                                                                                                                              |
| Video                | `libmpv2` (fork mantenido de libmpv-rs, requiere libmpv ≥ 2.0 / mpv ≥ 0.35)     | MPV resuelve demux/decode/HW-accel/subtítulos; nosotros solo consumimos frames vía su render API en modo software (ver sección 9.3).                                                                          |
| Audio                | `pipewire` (crate oficial de pipewire-rs) + `libspa`                            | Bindings seguros sobre `libpipewire`; `MainLoop`/`Context`/`Core`/`Registry` + `pipewire::channel` para comunicación entre hilos.                                                                             |
| FFT                  | `rustfft`                                                                       | 100% Rust, reemplaza FFTW3/kissfft sin FFI.                                                                                                                                                                   |
| Handoff audio→render | `arc-swap`                                                                      | Lectura sin bloqueo del último espectro calculado, desde el hilo de render.                                                                                                                                   |
| Config               | `serde` + `toml`                                                                | Manifiestos de wallpaper legibles a mano.                                                                                                                                                                     |
| IPC                  | `wallrs-proto` (serde) sobre socket Unix, JSON *newline-delimited*              | Simplicidad para v1; se puede migrar a un formato binario si el volumen de mensajes lo justifica (no se prevé).                                                                                               |
| CLI                  | `clap` (derive)                                                                 | Estándar de facto.                                                                                                                                                                                            |
| Logging              | `tracing` + `tracing-subscriber`                                                | Con dos hilos (render + audio) y fallos aislados por output, logs estructurados con spans facilitan depurar cuál output/hilo falló; preferible a `log`+`env_logger` para este caso.                           |

No se incluye `tokio`, `ffmpeg-next` (se prefiere delegar a MPV en v1 para no reimplementar sync a/v), ni `libpulse-binding` (fuera de alcance por decisión explícita).

---

## 6. Diseño del núcleo (`wallrs-core`)

### 6.1 Modelo de outputs y ciclo de frame

- Al arrancar, se enumeran los `wl_output` vía el registry de SCTK, complementados con `xdg-output` para nombre/posición/tamaño lógico (necesario para `--screen-span` estilo el proyecto original).
- Por cada output asignado a un wallpaper se crea:
  - Una superficie `wlr_layer_surface_v1` en capa `BACKGROUND`, ancla a las 4 esquinas, tamaño = tamaño del output, `exclusive_zone = -1`.
  - **Importante:** no se debe crear el `wgpu::Surface` hasta recibir el primer evento `configure` del layer-surface. Crear la superficie wgpu antes de conocer el tamaño real es un error común en clientes wlr-layer-shell nuevos — ver riesgo en sección 14.
  - Un `wgpu::Surface` (vía `raw-window-handle` del `wl_surface`) + su propio `SurfaceConfiguration`.
  - Una instancia del `WallpaperRenderer` correspondiente al tipo de contenido asignado.
- El render de cada output se dispara por el callback `wl_surface.frame()` de ese output (no por un timer fijo), de modo que cada monitor respeta su propia tasa de refresco. `--fps` (si se implementa) actúa como un límite superior, no como el disparador.
- Cada output es independiente: si su `WallpaperRenderer::render` retorna error, se loguea, se pausa ese output (pantalla en negro o última frame) y el resto del daemon sigue. Adicionalmente, envolver la llamada a `update`/`render` de cada output en `std::panic::catch_unwind` — un panic en un content-type (ej. un shader mal formado) no debe tumbar el proceso completo, solo ese output.

### 6.2 Protocolo de control (daemon + CLI)

Inspirado en el modelo de daemon+socket de `swww` (en vez del modelo "un proceso por invocación" del proyecto original de C++).

- Socket en `$XDG_RUNTIME_DIR/wallrs.sock`, registrado en `calloop` como fuente `Generic` sobre el fd de escucha.
- Mensajes definidos en `wallrs-proto`:

```rust
#[derive(Serialize, Deserialize)]
pub enum Command {
    SetWallpaper { output: OutputSelector, manifest_path: PathBuf },
    SetProperty  { output: OutputSelector, key: String, value: PropertyValue },
    Pause        { output: Option<String> },
    Resume       { output: Option<String> },
    Screenshot   { output: String, path: PathBuf },
    ListOutputs,
    Kill,
}

#[derive(Serialize, Deserialize)]
pub enum OutputSelector { Named(String), Span(Vec<String>), All }

#[derive(Serialize, Deserialize)]
pub enum PropertyValue { Bool(bool), Number(f32), Text(String), Color([f32; 4]) }
```

- `wallctl` es un binario sin estado: conecta, serializa un `Command` a JSON + `\n`, escribe, opcionalmente lee una respuesta, cierra.

### 6.3 Detección de fullscreen / pausa

- Vía `zwlr_foreign_toplevel_management_v1` (mismo protocolo que usa el proyecto original en su ruta Wayland). Al detectar una toplevel en estado fullscreen, se pausa el render de los outputs afectados (actualizable, análogo a `--fullscreen-pause-only-active` / `--fullscreen-pause-ignore-appid` del proyecto original, si se quiere paridad de features).

---

## 7. Contrato de content-types (`wallrs-render`)

```rust
pub trait WallpaperRenderer: Send {
    /// Se llama una vez al cargar el wallpaper en un output.
    fn init(&mut self, device: &wgpu::Device, queue: &wgpu::Queue,
            target_format: wgpu::TextureFormat) -> Result<(), RendererError>;

    /// Al cambiar el tamaño del output (o al arrancar).
    fn resize(&mut self, width: u32, height: u32);

    /// Una vez por frame, antes de render().
    fn update(&mut self, ctx: &FrameContext);

    /// Emite los comandos de render sobre la vista de la superficie del output.
    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView);

    /// Equivalente a --set-property: aplica un valor configurable en runtime.
    fn set_property(&mut self, _key: &str, _value: PropertyValue) -> Result<(), RendererError> {
        Ok(())
    }

    fn teardown(&mut self) {}
}

pub struct FrameContext<'a> {
    pub elapsed: Duration,
    pub delta: Duration,
    pub output_size: (u32, u32),
    pub pointer: Option<(f32, f32)>,   // None si --disable-mouse
    pub spectrum: Option<&'a [f32]>,   // None si no hay audio o está silenciado
}
```

`wallrs-render` también expone helpers comunes (compilar un pipeline de fullscreen-triangle, subir una textura RGBA8, un `Sampler` por defecto) para que los tres content-types no dupliquen boilerplate de wgpu.

---

## 8. Subsistema de audio (`wallrs-audio`) — solo PipeWire

### 8.1 Captura

- Hilo dedicado con su propio `pipewire::main_loop::MainLoop` (libpipewire no comparte loop fácilmente con `calloop`, así que se aísla).
- Se crea un `pw::stream::Stream` en dirección `Input`, apuntando al *monitor* del sink por defecto (equivalente a "escuchar lo que suena en el sistema", igual que hace PulseAudio hoy en el proyecto original). Selección de nodo vía metadata `default.audio.sink` de PipeWire.
- Formato negociado: `F32` intercalado, la tasa de muestreo que ofrezca el nodo (normalmente 48 kHz).
- Cada buffer de audio entrante se acumula en un ring buffer interno hasta juntar el tamaño de ventana FFT configurado (ej. 1024 o 2048 muestras).

### 8.2 Análisis espectral

- Ventana Hann sobre el bloque acumulado → `rustfft` → magnitud → bucketing logarítmico en `N` bandas configurables (equivalente a `barcount`/`frequency` del proyecto original, pero como parámetro simple del manifiesto, no como parte de un sistema de propiedades genérico).
- Salida: `Vec<f32>` normalizado de tamaño `N`, recalculado cada vez que se completa una ventana (no cada frame de video — el hilo de audio corre a su propio ritmo).

### 8.3 Modelo de hilos y entrega al render thread

```rust
pub struct SpectrumHandle(Arc<ArcSwap<Vec<f32>>>);

impl SpectrumHandle {
    pub fn latest(&self) -> Guard<Arc<Vec<f32>>> { self.0.load() }
}

pub fn spawn_capture(bands: usize) -> Result<(SpectrumHandle, JoinHandle<()>), AudioError>;
```

- El hilo de render nunca bloquea esperando audio: en cada frame lee el último valor disponible vía `ArcSwap::load()` (no hay mutex, no hay contención).
- Si PipeWire no está disponible, o el usuario pasa el equivalente a `--silent`/`--no-audio-processing`, `spawn_capture` simplemente no se llama y `FrameContext.spectrum` es `None` — los content-types deben degradar con gracia (sin barras de visualizador, sin reactividad), nunca deben asumir que `spectrum` siempre existe.

---

## 9. Content-types

### 9.1 Imagen/capas (`wallrs-content-image`) — Fase 3

- Cada capa es una textura cargada con `image`, dibujada como un quad con transform propio (posición, escala, offset por parallax en función de `ctx.pointer`, loop de paneo si el manifiesto lo pide).
- Sin sistema de partículas ni física. Si en el futuro se quiere más profundidad visual, se resuelve agregando más capas o pasando a un shader.

### 9.2 Shader estilo Shadertoy (`wallrs-content-shader`) — Fase 4

- Un único fragment shader (`.wgsl` nativo, o `.glsl` con un *shim* que envuelve `mainImage(out vec4 fragColor, in vec2 fragCoord)` en un `fragment main` compatible) sobre un fullscreen-triangle.
- Uniforms estándar expuestos: `time`, `resolution`, `mouse` (si no está deshabilitado), y opcionalmente una textura 1D de espectro (`spectrum_tex`) si el manifiesto pide `audio.reactive = true`.
- **Límite explícito de v1:** solo shaders de un solo paso (`mainImage`). Shaders de Shadertoy que usan múltiples buffers con feedback (Buffer A/B/C/D), cubemaps o input de teclado **no** son compatibles en v1 — documentarlo en el README del proyecto para que no se asuma compatibilidad total con Shadertoy.

### 9.3 Video (`wallrs-content-video`) — Fase 6

**Nota de investigación (verificada al escribir este documento, sept-2026):** mpv 0.41.0 (dic-2025) cambió el *video output* que usa MPV cuando **es dueño de su propia ventana** (`vo=gpu` → `vo=gpu-next`, basado en libplacebo), y ahora prefiere Vulkan para hw-decode sobre otras APIs. Esto es real, pero **no aplica a nuestra integración**: nosotros no le damos ventana a MPV, lo embebemos vía la *render API* pública (`render.h`), que a esta fecha solo expone dos backends: `MPV_RENDER_API_TYPE_OPENGL` (`render_gl.h`) y `MPV_RENDER_API_TYPE_SW`. Un backend `MPV_RENDER_API_TYPE_VULKAN` **no existe todavía** en mpv mainline — hay un issue abierto pidiéndolo ([mpv-player/mpv#18343](https://github.com/mpv-player/mpv/issues/18343), abierto el 4 de agosto de 2026 por un contribuidor externo, con un prototipo propio sin PR ni aval de los mantenedores, cero comentarios/labels/milestone al momento de revisarlo). Es una propuesta a seguir, no una API disponible — no diseñar la Fase 6 asumiendo que existe.

- Se usa la *render API* de `libmpv2` en modo **software** (`mpv_render_param` con `MPV_RENDER_API_TYPE_SW`), no el modo OpenGL. Esto evita tener que compartir contexto GL con el backend Vulkan de wgpu (interop GL↔Vulkan es frágil y específico de driver) — y es la única opción hoy que no requiere ese interop, dado que no hay backend Vulkan embebible.
- MPV entrega un frame RGBA en un buffer de CPU; se sube como `wgpu::Texture` cada vez que hay un frame nuevo (MPV avisa vía `MPV_RENDER_UPDATE_FRAME`).
- Costo conocido: hay una copia CPU→GPU por frame que no existiría con un path de zero-copy (DMA-BUF/hwdec). Se acepta ese costo en v1 a cambio de simplicidad; ver sección 14 para revisarlo si el perfilado muestra que es un cuello de botella real (por ejemplo, en 4K a 60 fps).
- Audio del video: MPV maneja su propia salida de audio (puede configurarse para salir directamente por PipeWire — el propio paquete de mpv en Debian se compila con `libpipewire-0.3-dev` y soporta el output driver `pipewire` nativamente) — no pasa por `wallrs-audio`.

---

## 10. Formato del manifiesto de wallpaper

Cada wallpaper es una carpeta con un `wallpaper.toml`. Ejemplos:

```toml
# --- shader ---
[wallpaper]
type = "shader"
name = "aurora"

[shader]
entry = "aurora.wgsl"

[shader.uniforms]
speed = 1.0

[audio]
reactive = true
bands = 32
```

```toml
# --- imagen con parallax ---
[wallpaper]
type = "image"
name = "valle"

[[image.layers]]
path = "fondo.png"

[[image.layers]]
path = "nubes.png"
parallax = 0.4
pan = { speed = 0.02, axis = "x" }
```

```toml
# --- video ---
[wallpaper]
type = "video"
name = "loop-lluvia"

[video]
path = "lluvia.mp4"
volume = 0.0
loop = true
```

Deliberadamente plano: no hay tipos de propiedad genéricos (`slider`, `combolist`, etc.) como en Wallpaper Engine. Si se necesita ese nivel de configurabilidad más adelante, es una extensión de este formato, no un rediseño.

---

## 11. Hoja de ruta por fases

### Fase 0 — Scaffolding
- [x] Workspace de Cargo con los 9 crates listados en la sección 4, cada uno compilando vacío.
- [x] CI (GitHub Actions): `cargo build`, `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`.
- [x] Licencia elegida y archivo `LICENSE` (ver sección 13).
- **Criterio de aceptación:** `cargo build --workspace` y `cargo test --workspace` pasan en CI sin implementación real.

### Fase 1 — Núcleo Wayland + wgpu
- [x] `wallrsd` detecta todos los outputs vía `wl_output` + `xdg-output`.
- [x] Crea una superficie `wlr-layer-shell` en capa `BACKGROUND` por output, esperando el primer `configure` antes de crear el `wgpu::Surface`.
- [x] Pinta un color sólido configurable a la tasa de refresco de cada output, dirigido por `frame` callbacks.
- **Criterio de aceptación:** correr en Sway y en Hyprland, con 1 y con 2 monitores conectados/desconectados en caliente, sin crashear el compositor ni el daemon. Verificar con `swaymsg -t get_outputs` (o equivalente) que el estado del compositor no se corrompe.

### Fase 2 — Daemon + CLI + multi-monitor
- [x] Socket de control funcionando, `wallctl set-color`, `wallctl list-outputs`, `wallctl kill` como comandos mínimos de humo.
- [x] Asignación de wallpaper por output vía selector (`--output <NAME>`, All, o Span).
- **Criterio de aceptación:** cambiar el color de un output específico en runtime sin reiniciar `wallrsd`.

### Fase 3 — Content-type imagen
- [ ] `wallrs-content-image` implementando el trait, con parallax basado en puntero.
- [x] `wallrs-content-image` implementando el trait, con parallax basado en puntero y desplazamiento continuo (pan).
- **Criterio de aceptación:** cargar un manifiesto de ejemplo con 2 capas y ver el parallax responder al mouse en tiempo real.

### Fase 4 — Content-type shader
- [ ] Carga de `.wgsl` nativo.
- [ ] Shim de compatibilidad `mainImage` → `fragment main` para `.glsl` estilo Shadertoy (un solo paso).
- **Criterio de aceptación:** correr 3 shaders públicos de Shadertoy (de un solo buffer) sin modificarlos más que pegar el código fuente.

### Fase 5 — Audio PipeWire
- [ ] `wallrs-audio` capturando el monitor del sink por defecto, FFT, `SpectrumHandle`.
- [ ] Wiring del espectro hacia `wallrs-content-shader` (textura de espectro) y opcionalmente hacia `wallrs-content-image` (ej. escala de una capa según graves).
- **Criterio de aceptación:** test unitario de `rustfft` + bucketing contra una señal seno conocida (verificar que el pico cae en la banda esperada). Test manual: reproducir música y ver el shader reaccionar.

### Fase 6 — Content-type video
- [ ] `wallrs-content-video` vía `libmpv2` en modo software, subida de frames a wgpu.
- **Criterio de aceptación:** reproducir un `.mp4` en loop sin drift audible de audio/video perceptible en 10 minutos continuos, en al menos un monitor 1080p60.

### Fase 7 — Fullscreen-pause + límites de FPS + screenshot
- [ ] Detector vía `zwlr_foreign_toplevel_management_v1`.
- [ ] `--fps` como techo, no como disparador.
- [ ] `wallctl screenshot` genera PNG del frame actual de un output.
- **Criterio de aceptación:** abrir un juego a pantalla completa y verificar que el uso de CPU/GPU de `wallrsd` cae a ~0 mientras dura.

### Fase 8 — Empaquetado y documentación
- [ ] Paquete AUR (o Nix flake), README con matriz de compositores probados, ejemplos de manifiestos.
- [ ] Unit de `systemd --user` para `wallrsd` (o instrucciones equivalentes para autostart en el compositor, ej. `exec` en la config de Sway).
- **Criterio de aceptación:** instalación limpia desde el paquete en una VM nueva, sin pasos manuales no documentados.

---

## 12. Estrategia de testing

- **Unitarios:** parsing de manifiestos TOML (round-trip), serialización de `wallrs-proto` (round-trip JSON), FFT/bucketing contra señales sintéticas conocidas.
- **Integración manual (matriz de compositores):** Sway, Hyprland, river como objetivos primarios (todos implementan `wlr-layer-shell`). GNOME queda fuera de alcance (no implementa el protocolo). KDE Plasma se marca como "no verificado" hasta probarlo empíricamente en Fase 1 — no asumir compatibilidad de antemano.
- **Hot-plug:** conectar/desconectar monitores en caliente durante la Fase 1 y de nuevo en la Fase 6 (con video corriendo).
- **Presupuesto de recursos (objetivo, a ajustar tras medir):** CPU idle < 2% con wallpaper de imagen estática en 1080p60; < 5% con shader típico; memoria residente < 150 MB sin contar buffers de video.
- **Verificación cuando el agente no tiene sesión gráfica real:** varios criterios de aceptación (Fases 1, 3, 4, 6, 7) requieren ver algo renderizado en un compositor real. Para smoke-tests automatizables sin display físico, usar el backend headless de wlroots (`WLR_BACKENDS=headless` con Sway, o un compositor anidado) y verificar por código de salida / logs / captura de framebuffer (`wallctl screenshot`, disponible desde Fase 7 — considerar adelantar una versión mínima a Fase 1 solo para testing). Lo que no se puede validar así (que el parallax "se sienta bien", que el shader se vea correcto, ausencia de drift audible en video) queda como checkpoint explícito de validación humana antes de cerrar esa fase — no asumir que pasar el smoke-test headless equivale a un "listo" de la fase.
- **Fixtures de prueba:** para la Fase 4, no versionar shaders copiados literalmente de Shadertoy en el repo (la mayoría son CC BY-NC-SA por defecto, lo que restringe redistribución) — escribir 2-3 shaders `mainImage` propios y simples como fixtures, y validar la compatibilidad con Shadertoy probando localmente (sin commitear) contra shaders públicos.

---

## 13. Requisitos no funcionales

- **Licencia:** a diferencia del proyecto original (GPL-3.0, justificado porque parseaba/reimplementaba lógica propietaria de Wallpaper Engine), `wallrs` no depende de ningún formato ni código de terceros protegido — se recomienda el estándar idiomático del ecosistema Rust, **dual MIT/Apache-2.0**. Único matiz a verificar antes de distribuir binarios: `mpv` puede compilarse en modo LGPLv2.1+ (`--enable-lgpl`) excluyendo componentes GPL-only; si se enlaza dinámicamente contra una build de mpv en modo GPL completo, la distribución del binario combinado puede quedar sujeta a esas condiciones. Confirmar la build de `libmpv` del sistema objetivo antes de empaquetar.
- **MSRV:** fijar una versión mínima de Rust en el `Cargo.toml` del workspace (recomendado: la más reciente estable al iniciar, dado que `wgpu` y `smithay-client-toolkit` evolucionan rápido).
- **Edición:** Rust 2021 o 2024, a elección del agente al iniciar Fase 0.

---

## 14. Riesgos y decisiones abiertas

| Riesgo / decisión abierta                                               | Impacto                                                      | Mitigación propuesta                                                                                                     |
| ----------------------------------------------------------------------- | ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------ |
| Crear el `wgpu::Surface` antes del primer `configure` del layer-surface | Crash o superficie con tamaño incorrecto                     | Esperar explícitamente el evento `configure` en Fase 1 antes de instanciar wgpu para ese output                          |
| Interop hilo de audio (PipeWire) ↔ hilo de render (calloop)             | Data races si se usa un mutex mal diseñado                   | `ArcSwap` para handoff sin bloqueo; nunca compartir el `MainLoop` de PipeWire con `calloop`                              |
| Rendimiento del path de video por software (copia CPU→GPU)              | Posible cuello de botella en 4K/60fps                        | Medir en Fase 6; si es un problema real, evaluar exportar DMA-BUF desde MPV para un path zero-copy como iteración futura |
| Soporte real de KDE Plasma para `wlr-layer-shell`                       | Puede no funcionar out-of-the-box                            | Verificar empíricamente en Fase 1, documentar el resultado en el README, no asumir compatibilidad                        |
| Compatibilidad parcial con Shadertoy (solo un paso)                     | Expectativas de usuarios que prueben shaders multi-buffer    | Documentar el límite explícitamente desde la Fase 4                                                                      |
| Extensión futura a wallpapers web                                       | Mayor complejidad (offscreen render de un webview a textura) | Explícitamente fuera de v1; si se retoma, evaluar `wry`/WebKitGTK primero por ser más liviano que CEF                    |

---

## 15. Apéndice — crates de referencia (verificados)

| Crate                    | Versión de referencia                                           | Notas                                                                                                                |
| ------------------------ | --------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `smithay-client-toolkit` | 0.21.x                                                          | Features `calloop` habilita `calloop` + `calloop-wayland-source` como reexports                                      |
| `wgpu`                   | según lo que use SCTK en sus propios dev-deps como piso (0.19+) | Backend Vulkan en Linux                                                                                              |
| `pipewire`               | 0.10.x                                                          | Bindings oficiales de pipewire-rs; `MainLoop`/`Context`/`Core`/`Registry`, más `pipewire::channel` para cruzar hilos |
| `libspa`                 | acompaña a `pipewire`                                           | Negociación de formato de stream                                                                                     |
| `libmpv2`                | 5.0.x                                                           | Fork mantenido de libmpv-rs, requiere libmpv ≥ 2.0 / mpv ≥ 0.35; módulo `render` para render API custom              |
| `rustfft`                | última estable                                                  | FFT puro Rust                                                                                                        |
| `arc-swap`               | última estable                                                  | Handoff sin bloqueo                                                                                                  |
| `image`                  | última estable                                                  | Decodificación de imágenes                                                                                           |
| `clap`                   | última estable, modo derive                                     | CLI de `wallctl`                                                                                                     |
| `serde` + `toml`         | última estable                                                  | Manifiestos y config                                                                                                 |