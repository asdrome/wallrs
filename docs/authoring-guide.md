# Guía de Creación de Fondos para wallrs (Authoring Guide) 🎨

Esta guía explica cómo diseñar, estructurar y optimizar fondos animados para **`wallrs`**, aprovechando sus tres motores de contenido (**imágenes con parallax**, **shaders procedurales WGSL/Shadertoy**, y **video**) junto con el sistema de **audio ambiental independiente** y **reactividad por PipeWire**.

---

## 📁 Anatomía de un Wallpaper

En `wallrs`, cada fondo es un **directorio autocontenido** que incluye:
1. Un archivo de manifiesto obligatorio llamado **`wallpaper.toml`**.
2. Todos los recursos referenciados (imágenes, shaders, videos o pistas de audio) almacenados en rutas relativas al manifiesto.

```text
mi-fondo/
├── wallpaper.toml       # Manifiesto principal
├── fondo.png            # Capa base
├── nubes.png            # Capa con parallax/pan
└── ambiente.ogg         # (Opcional) Pista de audio ambiental
```

---

## 🛠️ Herramienta de Validación (`wallctl validate`)

Antes de cargar o compartir un fondo, puedes verificar que todos sus archivos existan, que el manifiesto sea sintácticamente correcto y que los shaders compilen sin errores ejecutando:

```bash
wallctl validate mi-fondo/
```

Ejemplo de salida:
```text
Validating wallpaper: "mi-fondo/wallpaper.toml"
  • Name: "mi-fondo"
  • Type: "image"
  • Layers (2):
    ✓ Layer 0: "fondo.png" (1920x1080) [parallax: none, pan: none]
    ✓ Layer 1: "nubes.png" (1920x1080) [parallax: 0.40, pan: speed: 0.01, axis: x]
  • Background audio track: "ambiente.ogg" [volume: 40%, loop: true]
✓ Wallpaper 'mi-fondo' is valid and ready to use!
```

---

## 1. Fondos de Imagen y Parallax (`type = "image"`)

Permite componer múltiples capas en orden de profundidad, con soporte para seguimiento del cursor del ratón (parallax) y desplazamiento continuo automático (*pan*).

### Estructura en `wallpaper.toml`

```toml
[wallpaper]
type = "image"
name = "cyber-city"
author = "Tu Nombre"
description = "Ciudad cyberpunk con efecto parallax y lluvia"

# Capa de fondo estática (cielo/estrellas)
[[image.layers]]
path = "sky.png"

# Capa intermedia con paneo automático constante hacia la derecha
[[image.layers]]
path = "fog.png"
pan = { speed = 0.005, axis = "x" }

# Capa frontal con seguimiento del ratón (parallax)
[[image.layers]]
path = "buildings.png"
parallax = 0.35
```

### Opciones de cada capa (`[[image.layers]]`)

| Propiedad  | Tipo              | Descripción                                                                     |
| :--------- | :---------------- | :------------------------------------------------------------------------------ |
| `path`     | String            | Ruta al archivo de imagen relativo a `wallpaper.toml` (PNG, JPEG, WebP).        |
| `parallax` | Float (opcional)  | Intensidad del movimiento según la posición del cursor (0.1 a 0.8 recomendado). |
| `pan`      | Objeto (opcional) | Desplazamiento continuo: `{ speed = 0.01, axis = "x" }` o `"y"`.                |

### 💡 Consejos de diseño para parallax:
- **Formatos:** Usa **WebP** o **PNG optimizado** con transparencia alfa (RGBA).
- **Margen de seguridad:** Si una capa tiene un factor de `parallax = 0.5`, se desplazará ligeramente con el ratón. Diseña esa capa entre un **5% y 10% más grande** que la resolución base (ej. `2048x1152` para pantallas `1920x1080`), o mantén los elementos críticos centrados para evitar bordes vacíos.
- **Jerarquía:** Asigna valores de `parallax` más bajos a planos lejanos (ej. `0.1` - `0.2`) y valores más altos a planos cercanos al espectador (ej. `0.4` - `0.7`).

---

## 2. Shaders Procedurales (`type = "shader"`)

`wallrs` ejecuta shaders acelerados directamente por hardware en **Vulkan / WGPU** con soporte para WGSL nativo o GLSL al estilo Shadertoy.

### Estructura en `wallpaper.toml`

```toml
[wallpaper]
type = "shader"
name = "aurora-boreal"
author = "Tu Nombre"

[shader]
entry = "aurora.wgsl"
audio = true  # (Opcional) Si es true, conecta PipeWire para alimentar u_params.audio_*
```

### Variables Uniforms Disponibles en WGSL:

Si defines el struct `ShaderUniforms`, `wallrs` le inyectará automáticamente los siguientes datos en cada fotograma (`@group(0) @binding(0)`):

```wgsl
struct ShaderUniforms {
    resolution: vec2<f32>,                 // Ancho y alto del monitor en píxeles
    time: f32,                             // Tiempo transcurrido en segundos
    time_delta: f32,                        // Delta de tiempo desde el último fotograma
    mouse: vec4<f32>,                      // xy: coordenadas del ratón, zw: botones
    frame: u32,                            // Contador de fotogramas renderizados
    custom0: f32,                          // Propiedades dinámicas configurables vía wallctl
    custom1: f32,
    custom2: f32,
    audio_bass: f32,                       // Reactividad de graves PipeWire [0.0..1.0]
    audio_mid: f32,                        // Reactividad de medios [0.0..1.0]
    audio_treble: f32,                     // Reactividad de agudos [0.0..1.0]
    audio_volume: f32,                     // Nivel de volumen general [0.0..1.0]
    audio_spectrum: array<vec4<f32>, 8>,   // 32 bandas de frecuencia FFT logarítmicas
};

@group(0) @binding(0) var<uniform> u_params: ShaderUniforms;
```

> **Nota:** Si tu archivo `.wgsl` contiene únicamente la función de fragmento `fs_main`, `wallrs` inyectará automáticamente el vertex shader de pantalla completa y la cabecera de uniforms.

### Compatibilidad con Shadertoy (GLSL):
Si tu archivo tiene extensión `.glsl` o contiene `void mainImage(out vec4 fragColor, in vec2 fragCoord)`, `wallrs` lo traduce automáticamente a WGSL mediante Naga:

```glsl
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float wave = sin(uv.x * 10.0 + iTime) * 0.5 + 0.5;
    // iBass, iMid, iTreble, iVolume están disponibles automáticamente
    fragColor = vec4(uv.x * iBass, wave, uv.y, 1.0);
}
```

---

## 3. Fondos de Video (`type = "video"`)

Reproducción eficiente de video en bucle impulsada por `libmpv2` con decodificación acelerada.

### Estructura en `wallpaper.toml`

```toml
[wallpaper]
type = "video"
name = "lofi-desk"

[video]
path = "video.mp4"
volume = 45.0      # Volumen inicial de 0.0 a 100.0 (opcional, por defecto: 50.0)
loop = true        # Repetición en bucle infinito (opcional, por defecto: true)
```

### 💡 Consejos para videos:
- **Codecs recomendados:** **H.264** (`.mp4`) o **VP9 / AV1** (`.webm`).
- **Resolución:** `1920x1080` a `30fps` o `60fps` ofrece el mejor equilibrio entre calidad visual y uso mínimo de recursos de GPU.
- **Bucle perfecto (*Seamless Loop*):** Asegúrate de que el último fotograma del video coincida con el primero para evitar saltos perceptibles al reiniciar el bucle.

---

## 4. Audio Ambiental Independiente (`[audio]`)

Puedes añadir una pista de audio ambiental o música en bucle a **cualquier fondo de imagen o shader**:

```toml
[wallpaper]
type = "image"
name = "lluvia-en-el-bosque"

[[image.layers]]
path = "bosque.png"

# Audio de fondo independiente
[audio]
path = "lluvia_suave.ogg"
volume = 35.0   # Nivel de volumen de 0.0 a 100.0
loop = true     # Reproducción continua en bucle
```

### Características del audio independiente:
- **Formatos:** Compatible con OGG Vorbis, MP3, FLAC, WAV, AAC y Opus.
- **Consumo mínimo:** Se ejecuta en modo headless (`vo=null`, `video=no`), con un impacto de CPU prácticamente nulo (~0%).
- **Pausado inteligente:** Al maximizar una ventana, abrir un juego a pantalla completa o ejecutar `wallctl toggle-pause`, el audio se silencia/pausa automáticamente y se reanuda al volver al escritorio.
- **Sinergia con Shaders:** Si usas `[audio]` en un fondo de tipo `shader` con `audio = true`, la música emitida saldrá a PipeWire y el analizador de espectro de `wallrs` la capturará para alimentar las barras y ondas reactivas en tiempo real.

---

## 5. Pruebas y Uso en Vivo

1. Valida el fondo:
   ```bash
   wallctl validate ~/MisFondos/cyber-city
   ```
2. Cárgalo en el daemon:
   ```bash
   wallctl set-wallpaper ~/MisFondos/cyber-city
   ```
3. Ajusta propiedades en tiempo real:
   ```bash
   wallctl set-property volume 20.0
   ```
4. Pausa y reanuda con un atajo:
   ```bash
   wallctl toggle-pause
   ```

