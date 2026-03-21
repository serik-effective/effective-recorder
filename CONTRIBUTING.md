# Разработка

> [English version](CONTRIBUTING.en.md)

## Стек

| Слой | Технология |
|------|-----------|
| Фреймворк | Tauri 2 |
| Фронтенд | SvelteKit 2 + Svelte 5 |
| Бэкенд | Rust |
| Видео | FFmpeg (статическая линковка через ffmpeg-next) |
| Захват экрана | xcap + CoreGraphics (macOS) |
| Захват звука | cpal |
| Транскрибация | whisper-rs (Whisper.cpp) |

## Быстрый старт

```bash
npm install
npm run tauri dev          # разработка (горячая перезагрузка JS)
npm run tauri build --debug  # сборка .app бандла (для тестирования пермишенов)
```

> **Важно:** `tauri dev` запускает бинарник без бандла — macOS не покажет диалоги разрешений (камера, экран). Для тестирования пермишенов используйте `tauri build --debug`.

## Структура проекта

```
src/                              # Фронтенд (SvelteKit + Svelte 5)
  routes/
    +page.svelte                  # Главный UI
    camera-overlay/+page.svelte   # Окно камеры

src-tauri/                        # Бэкенд (Rust + Tauri 2)
  src/
    lib.rs                        # Точка входа, Tauri-команды
    recorder.rs                   # Оркестрация записи (потоки)
    encoder.rs                    # FFmpeg кодирование, пресеты
    audio.rs                      # Захват звука (cpal)
    screen.rs                     # Захват экрана (xcap / CoreGraphics)
    transcription.rs              # Whisper распознавание речи
    transcription_queue.rs        # Очередь транскрибации
    history.rs                    # История записей
    settings.rs                   # Настройки приложения
    permissions.rs                # Проверка пермишенов macOS
  capabilities/default.json       # Tauri-пермишены для окон
  Info.plist                      # macOS описания использования
  Entitlements.plist              # Энтайтлменты камеры и микрофона
  tauri.conf.json                 # Конфигурация приложения
  tauri.conf.qa.json              # Конфигурация QA-бандла
```

## Архитектура

### Пайплайн записи

При старте записи создаются три потока:
1. **Захват экрана** — на macOS через CGDisplayCreateImage (~7мс/кадр), на других платформах через xcap
2. **Захват звука** — через cpal с выбранного устройства ввода
3. **FFmpeg энкодер** — кодирует кадры + аудио в H.264 MP4

Потоки связаны через bounded crossbeam-каналы (30 для видео, 200 для аудио).

### Транскрибация

После остановки записи файл ставится в очередь на транскрибацию через Whisper.cpp. Очередь персистентна — переживает перезапуски приложения. Поддерживает retry и cancel.

### Камера

Отдельное окно Tauri (`camera-overlay`) с `always_on_top`, показывающее веб-камеру. Оверлей захватывается как часть записи экрана — никакого отдельного композитинга.

## Bundle ID

| Среда | Идентификатор | Назначение |
|-------|-------------|------------|
| Dev | `com.effective-recorder.dev` | Ежедневная разработка |
| QA | `com.effective-recorder.qa` | Чистое тестирование |
| Release | `com.effective-recorder.app` | Продакшн |

## Сброс пермишенов macOS

```bash
tccutil reset Camera com.effective-recorder.dev
tccutil reset Microphone com.effective-recorder.dev
tccutil reset ScreenCapture com.effective-recorder.dev
```

## Статическая сборка FFmpeg

Для standalone-бандла без внешних зависимостей:

```bash
bash build-ffmpeg-static.sh
export FFMPEG_DIR=$PWD/ffmpeg-build/install
export PKG_CONFIG_PATH=$FFMPEG_DIR/lib/pkgconfig
npm run tauri build -- --debug
```

## CI/CD

GitHub Actions собирает бандлы для 4 платформ:
- macOS ARM64 (Apple Silicon)
- macOS x64 (Intel)
- Windows x64
- Linux x64

Сборка запускается вручную или при пуше тегов `v*`.
