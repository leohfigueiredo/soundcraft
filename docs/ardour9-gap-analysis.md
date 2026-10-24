# Ardour 9 vs Soundcraft — Gap Analysis Completo

**Data:** 2025-01-XX  
**Source:** Ardour 9.8.0 (binário em `/home/leonardo/opt/Ardour_x86_64-9.8.0-demo/bin/ardour-9.8.0`)  
**Method:** Reverse engineering via `strings`, Lua scripts analysis, plugin surface enumeration

---

## 🔴 CRITICAL GAPS — Falta no Soundcraft

### 1. Video Module (HIGH PRIORITY)
O Ardour 9 tem um módulo de vídeo completo que o Soundcraft NÃO tem:

| Feature | Status Ardour 9 | Status Soundcraft |
|---------|-----------------|-------------------|
| Video Timeline widget | ✅ `VideoTimeLine` | ❌ Não existe |
| Video Monitor (preview) | ✅ `VideoMonitor` | ❌ Não existe |
| Add Video to Session | ✅ `AddVideoDialog` | ❌ Não existe |
| Export to .mp4/.h264 | ✅ `ExportVideoDialog` | ❌ Não existe |
| Transcode video to session | ✅ `TranscodeVideoDialog` | ❌ Não existe |
| Video Server (HTTP streaming) | ✅ `VideoServerDialog` (port 1554) | ❌ Não existe |
| Chapter marks for MP4 | ✅ `export.mp4` + chapter support | ❌ Não existe |
| Video pull-up/down (frame rate match) | ✅ Pull-Up/Down toggle | ❌ Não existe |
| External timecode sync with video | ✅ Genlock, LTC, MTC | ⚠️ Parcial |

**Código fonte Ardour 9:**
- `../gtk2_ardour/video_monitor.cc`
- `../gtk2_ardour/video_timeline.cc`
- `../gtk2_ardour/export_video_dialog.cc`
- `../gtk2_ardour/transcode_video_dialog.cc`

### 2. VCA System (HIGH PRIORITY)
O Ardour 9 tem um sistema completo de VCAs que o Soundcraft NÃO tem:

| Feature | Status Ardour 9 | Status Soundcraft |
|---------|-----------------|-------------------|
| VCA Master Strip | ✅ `VCAMasterStrip` | ❌ Não existe |
| VCA Time Axis View | ✅ `VCATimeAxisView` | ❌ Não existe |
| Assign tracks to VCA | ✅ "Assign Selection to VCA..." | ❌ Não existe |
| Multiple VCAs | ✅ N VCAs com cor por faixa | ❌ Não existe |
| VCA color palette | ✅ `use-palette-for-new-vca` | ❌ Não existe |
| Hide/show VCA strips | ✅ `show-vca-pane` | ❌ Não existe |

### 3. MIDI Quantization & Swing (MEDIUM PRIORITY)
O Ardour 9 tem quantização MIDI avançada que o Soundcraft NÃO tem:

| Feature | Status Ardour 9 | Status Soundcraft |
|---------|-----------------|-------------------|
| Global Quantize dialog | ✅ `QuantizeDialog` + `QuantizeWidget` | ⚠️ Básico |
| Swing/Groove | ✅ "Swing" parameter | ❌ Não existe |
| MIDI grid snapping | ✅ `snap-to-region-*` | ⚠️ Parcial |
| Note position edit | ✅ Move start/end earlier/later (fine) | ⚠️ Limitado |
| Pitchbend quantization | ✅ `PitchColorDialog` | ❌ Não existe |

---

## 🟡 MEDIUM GAPS

### 4. Timecode Advanced Features
- **LTC continuous output** — Ardour continua a enviar LTC quando o transport está parado (`ltc-send-continum`)
- **Genlock** — External timecode assumed to be sample-clock synced (`timecode-sync-fr`)
- **29.97 fps mode** — Non-drop vs drop-frame (`timecode-sync-fr`)

### 5. Track Grouping / Subgroup Buses
- **Add New Subgroup Bus** — Ardour tem "Add New Subgroup Bus" no menu de criação
- **Group to VCA assignment** — `Assign Group to VCA...`
- **Remove Subgroup Bus** — `Remove Subgroup Bus`

### 6. Region Envelope Editing
O Ardour tem edição avançada de envelopes de região via Lua API:
- `s_region_envelope.lua` — Evaluate region gain curve
- `s_plugin_automation.lua` — Plugin automation per-region
- `s_fader_automation.lua` — Fader automation with move support

### 7. BBT Markers (Beat/Bar/Time)
- **BBT markers** — Ardour tem "add/remove BBT marker" e "move BBT point"
- **Display Navigation Timeline** — Alternativa ao timeline tradicional
- **Time vs Beat time distinction** — "If you choose Audio Time, these items will not move when you change tempo"

---

## 🟢 LOW PRIORITY (Nice to Have)

### 8. MIDI Patch Bank Management
- `PatchBankList`, `PatchChangeTab`, `PatchChangeGridDialog`
- `PatchChangeTriggerWindow`
- `s_plugin_reorder.lua` — Plugin order reverse

### 9. Video Server Advanced Setup
- Remote video server URL configuration
- Custom docroot path
- "Advanced Setup (remote video server)" option

### 10. Region Fade Options
- Use region fades vs transport fades
- Monitor fade settings
- Crossfade and fade-in/fade-out per-region

---

## 📊 Prioritização por Complexidade

| Fase | Feature | Estimativa | Impacto |
|------|---------|------------|---------|
| **Quick Win 1** | BBT Markers | 2h | ⭐⭐ |
| **Quick Win 2** | Region Envelope API | 4h | ⭐⭐⭐ |
| **Médio 1** | VCA Master Strip (egui) | 8-12h | ⭐⭐⭐⭐ |
| **Médio 2** | MIDI Swing/Groove | 6h | ⭐⭐⭐ |
| **Hard 1** | Video Timeline + Monitor | 40-60h | ⭐⭐⭐⭐⭐ |
| **Hard 2** | Export to .mp4 (ffmpeg) | 30-50h | ⭐⭐⭐⭐ |

---

## 🎯 Recomendação de Implementação

1. **Começar pelo Quick Win 1 — BBT Markers**: É simples, já temos a estrutura de markers no Soundcraft, só precisa adicionar o tipo BBT e a lógica de snapping por beat/bar/time.

2. **Depois Quick Win 2 — Region Envelope API**: Expor a API Lua do Ardour para edição de envelopes por região.

3. **Fase Médio — VCA System**: O mais impactante para o mixer. Começar com um VCA Master Strip em egui que pode controlar volume/mute/solo de múltiplas faixas.

4. **Fase Hard — Video Module**: O maior gap mas também o mais complexo. Requer integração com ffmpeg, server HTTP e UI de timeline de vídeo.
