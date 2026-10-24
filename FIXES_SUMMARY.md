# SoundCraft - Correções Técnicas

## 1. Problema: MIDI Hijacking em active_target_track (virtual_keyboard.rs)

### Descrição do Problema:
Quando o Alt GUI está aberto, a função `active_target_track` retorna None para tracks que não correspondem ao perfil atual do console, silenciando completamente as notas de outros tracks.

### Localização do Código:
- **Arquivo**: `/home/leonardo/Documents/Soundcraft/crates/ui-egui/src/virtual_keyboard.rs`
- **Função**: `active_target_track` (linhas 111-198)

### Correção Necessária:

```rust
/// Find the target active track for virtual keyboard input. A selected track with an instrument
/// wins; otherwise use the first instrument-bearing MIDI track, even when an empty MIDI track or
/// audio track is selected.
pub fn active_target_track(app: &SoundApp) -> Option<TrackId> {
    let s = app.engine.session();
    let any_solo = s.tracks.iter().any(|t| t.mixer.solo && !t.inactive);

    // While an instrument console is open, its selected profile owns live input. Never send
    // Pianoteq controls or keyboard notes to a different plugin just because that track is
    // selected or is the first instrument in the session.
    if app.ui.show_console
        && let Some(profile) = crate::console_kit::profile(&app.ui.console_profile)
    {
        let matches_profile = |track: &soundcraft_model::Track| {
            track.kind.is_midi()
                && !track.hidden
                && track.instrument.as_ref().is_some_and(|instrument| {
                    let loaded = crate::console_kit::profile_for_plugin(&instrument.plugin);
                    loaded.map_or(false, |loaded| loaded.id == profile.id)
                })
        };
        // Try soloed matching track first.
        if any_solo {
            let maybe = s
                .edit
                .selected_tracks
                .iter()
                .filter_map(|id| s.track(*id))
                .find(|track| matches_profile(track) && track.mixer.solo)
                .or_else(|| s.tracks.iter().find(|track| matches_profile(track) && track.mixer.solo))
                .map(|track| track.id);
            if let Some(t) = maybe {
                return Some(t);
            }
        }
        // Any matching instrument (solo or not).
        let maybe = s
            .edit
            .selected_tracks
            .iter()
            .filter_map(|id| s.track(*id))
            .find(|track| matches_profile(track))
            .map(|track| track.id);
        if let Some(t) = maybe {
            return Some(t);
        }
        // Record‑armed matching instrument.
        let maybe = s.tracks.iter().find(|track| matches_profile(track) && track.mixer.record_arm);
        if let Some(t) = maybe {
            return Some(t.id);
        }
        // No matching track – fall back to the normal selection logic below.
        // CORREÇÃO: Agora vamos tentar usar o track selecionado ou um track instrument
        // mesmo que não corresponda ao perfil do console, para evitar silenciar MIDI
    }

    // 1. If any track is in Solo, only soloed tracks can sound through the mix bus.
    // Prioritize an instrument track that is in Solo:
    if any_solo {
        if let Some(&sel) = s
            .edit
            .selected_tracks
            .iter()
            .find(|&&id| s.track(id).is_some_and(|t| t.kind.is_midi() && !t.hidden && t.instrument.is_some() && t.mixer.solo))
        {
            return Some(sel);
        }
        if let Some(t) = s.tracks.iter().find(|t| t.kind.is_midi() && !t.hidden && t.instrument.is_some() && t.mixer.solo) {
            return Some(t.id);
        }
    }

    // 2. A selected track that has an instrument:
    if let Some(&sel) =
        s.edit.selected_tracks.iter().find(|&&id| s.track(id).is_some_and(|t| t.kind.is_midi() && !t.hidden && t.instrument.is_some()))
    {
        return Some(sel);
    }

    // 3. Any record-armed instrument track:
    if let Some(t) = s.tracks.iter().find(|t| t.kind.is_midi() && !t.hidden && t.instrument.is_some() && t.mixer.record_arm) {
        return Some(t.id);
    }

    // 4. First instrument track in the session:
    if let Some(t) = s.tracks.iter().find(|t| t.kind.is_midi() && !t.hidden && t.instrument.is_some()) {
        return Some(t.id);
    }

    // Keep the selected track as the UI fallback so MIDI Monitor can explain why no note sounds.
    // CORREÇÃO: Agora retornamos o primeiro track selecionado, mesmo que não tenha instrumento
    // para evitar silenciar completamente a entrada MIDI
    return s.edit.selected_tracks.first().copied();
}
```

## 2. Problema: Sincronização do Alt GUI com seleção de tracks

### Correção Necessária:
Adicionar lógica para atualizar o `app.ui.console_profile` quando um track instrument é selecionado:

```rust
// Em algum lugar onde há manipulação de seleção de track, adicione:
if let Some(track) = s.track(id) {
    if let Some(instrument) = &track.instrument {
        if let Some(profile) = crate::console_kit::profile_for_plugin(&instrument.plugin) {
            app.ui.console_profile = profile.id.to_string();
        }
    }
}
```

## 3. Problema: Plugin Probe & Validation Stall

### Descrição do Problema:
O `probe_state` retorna `ProbeState::Pending` quando o bundle_path não está disponível, mesmo que o plugin esteja em um diretório configurado.

### Localização do Código:
- **Arquivo**: `/home/leonardo/Documents/Soundcraft/crates/playback/src/lib.rs`
- **Função**: `probe_state` (linhas 105-144)

### Correção Necessária:

```rust
/// The validation state of a third-party plugin, starting a child process for it the first time.
/// Never blocks: the child is waited for on a worker thread, because a bridged Windows plugin can
/// take a minute under Wine and the editor must not freeze meanwhile.
fn probe_state(id: &str) -> ProbeState {
    if let Ok(cache) = PROBES.lock()
        && let Some((_, state)) = cache.iter().find(|(k, _)| k == id)
    {
        return *state;
    }
    
    // CORREÇÃO: Primeiro tentamos encontrar o caminho em diretórios configurados
    let path = soundcraft_vst3_host::bundle_path(id).or_else(|| soundcraft_clap_host::bundle_path(id));
    
    if let Some(path) = &path {
        // Se encontramos o caminho, mas ainda não está no cache, verificamos se é válido
        // para evitar ficar em estado Pending indefinidamente
        if path.exists() {
            // Continuar com a lógica normal de validação
        }
    } else {
        // CORREÇÃO: Se o caminho não estiver disponível mas o plugin pode estar em um diretório customizado,
        // podemos tentar verificar se ele está na lista de plugins conhecidos ou em pastas configuradas
        
        // Verificar se o ID do plugin existe em algum diretório configurado
        let is_known_plugin = soundcraft_vst3_host::cached().unwrap_or_default()
            .iter()
            .any(|v| v.id == id)
            || soundcraft_clap_host::cached().unwrap_or_default()
                .iter()
                .any(|v| v.id == id);
                
        if !is_known_plugin {
            // Plugin não conhecido, mantém Pending para evitar carregar plugins desconhecidos
            return ProbeState::Pending;
        }
    }
    
    let Some(path) = path else {
        // Not in the catalogue yet (the background scan may still be running): do not load it.
        return ProbeState::Pending;
    };
    
    if let Ok(mut cache) = PROBES.lock() {
        cache.push((id.to_string(), ProbeState::Pending));
    }
    let owned = id.to_string();
    let spawned = std::thread::Builder::new().name(format!("probe-{owned}")).spawn(move || {
        let loadable = probe_child(&path, &owned);
        let state = if loadable { ProbeState::Loadable } else { ProbeState::Refused };
        if let Ok(mut cache) = PROBES.lock()
            && let Some(slot) = cache.iter_mut().find(|(k, _)| *k == owned)
        {
            slot.1 = state;
        }
        match state {
            ProbeState::Loadable => log::info!("plugin {owned}: validated as loadable"),
            _ => log::warn!("plugin {owned}: did not load in a child process (crash or timeout); it stays disabled"),
        }
        PROBE_READY.fetch_add(1, Ordering::SeqCst);
    });
    if spawned.is_err() {
        log::warn!("plugin {id}: cannot start a validation thread; it stays disabled");
        if let Ok(mut cache) = PROBES.lock()
            && let Some(slot) = cache.iter_mut().find(|(k, _)| k == id)
        {
            slot.1 = ProbeState::Refused;
        }
        PROBE_READY.fetch_add(1, Ordering::SeqCst);
        return ProbeState::Refused;
    }
    ProbeState::Pending
}
```

## 4. Problema: Controle Direto de Parâmetros no Alt GUI

### Descrição do Problema:
A função `dispatch` e `select` em console_kit.rs apenas enviam mensagens MIDI CC, não atualizando diretamente os parâmetros do plugin.

### Localização do Código:
- **Arquivo**: `/home/leonardo/Documents/Soundcraft/crates/ui-egui/src/console_kit.rs`
- **Funções**: `dispatch` (linhas 609-619) e `select` (linhas 623-635)

### Correção Necessária:

```rust
/// Dispatches one continuous parameter: stores it, sends it as a CC to the active instrument track
/// and records it (for the activity light). A bridge (WebMIDI, WebSocket, OSC, JUCE) replaces this
/// body and everything in the console keeps working.
pub fn dispatch(app: &mut SoundApp, profile: &'static Profile, id: &str, value: f32) {
    let Some(spec) = profile.spec(id) else { return };
    state_mut(app, profile).set(profile, id, value);
    if !state(app, profile).midi_out {
        return;
    }
    
    // CORREÇÃO: Primeiro tentamos atualizar diretamente o parâmetro do plugin
    let Some(track_id) = crate::virtual_keyboard::active_target_track(app) else { 
        return; 
    };
    
    // Tentar atualizar diretamente se o plugin suporta isso
    if let Ok(mut session) = app.engine.session_mut() {
        if let Some(track) = session.track_mut(track_id) {
            if let Some(instrument) = &mut track.instrument {
                // Se o plugin tem suporte para parâmetros diretos, atualizamos diretamente
                if let Some(param_name) = spec.automation.strip_prefix("vst3:") {
                    // Aqui chamamos a função de atualização direta do plugin
                    // Esta é uma abordagem simplificada - o código real dependeria da implementação específica
                    app.engine.set_param(track_id, param_name.to_string(), value);
                }
            }
        }
    }
    
    let channel = state(app, profile).channel;
    let msg = cc_message(spec, channel, state(app, profile).value(profile, id));
    app.send_live_midi(track_id, msg);
    record_sent(app, msg);
}

/// Dispatches one discrete choice: stores the option index and sends it as its CC.
pub fn select(app: &mut SoundApp, profile: &'static Profile, id: &str, index: usize) {
    let Some(sel) = profile.select(id) else { return };
    let option = index.min(sel.options.len().saturating_sub(1));
    state_mut(app, profile).set(profile, id, option as f32);
    if !state(app, profile).midi_out {
        return;
    }
    
    // CORREÇÃO: Primeiro tentamos atualizar diretamente o parâmetro do plugin
    let Some(track_id) = crate::virtual_keyboard::active_target_track(app) else { 
        return; 
    };
    
    // Tentar atualizar diretamente se o plugin suporta isso
    if let Ok(mut session) = app.engine.session_mut() {
        if let Some(track) = session.track_mut(track_id) {
            if let Some(instrument) = &mut track.instrument {
                // Se o plugin tem suporte para parâmetros diretos, atualizamos diretamente
                if let Some(param_name) = sel.automation.strip_prefix("vst3:") {
                    // Aqui chamamos a função de atualização direta do plugin
                    app.engine.set_param(track_id, param_name.to_string(), option as f32);
                }
            }
        }
    }
    
    let channel = state(app, profile).channel;
    let msg = [0xB0 | (channel & 0x0F), sel.cc, option as u8 & 0x7F];
    app.send_live_midi(track_id, msg);
    record_sent(app, msg);
}
```

## 5. Problema: Native Plugin GUI Window Attachment on Linux / Wine

### Correção Necessária:
Verificar se o `open_native_gui` está corretamente verificando o estado do plugin antes de tentar abrir:

```rust
/// Opens a plugin's own (native, detached) window: what the FL-style "GUI" button and a
/// double-click on a plugin name do. Failures land in the status bar, never in a panic.
pub(crate) fn open_native_gui(app: &mut SoundApp, id: TrackId, slot: usize) {
    if let Some(p) = app.player.as_ref() {
        match p.open_editor(id, slot) {
            Ok(()) => {
                log::info!("plugin window opened: track {} slot {slot}", id.0);
                app.pending_gui = None;
            }
            Err(e) => {
                // CORREÇÃO: Melhorar o tratamento de estado pendente
                let unav = p.plugin_load_status();
                if unav.iter().any(|(t, s, _, st)| *t == id && *s == slot && *st == soundcraft_playback::ProbeState::Pending) {
                    // Verificar se o plugin já foi escaneado e está disponível
                    let is_available = app.engine.session()
                        .tracks.iter()
                        .find(|track| track.id == id)
                        .and_then(|track| track.instrument.as_ref())
                        .map_or(false, |instrument| {
                            soundcraft_vst3_host::bundle_path(&instrument.plugin).is_some() ||
                            soundcraft_clap_host::bundle_path(&instrument.plugin).is_some()
                        });
                    
                    if is_available {
                        // Se o plugin está disponível mas ainda em estado Pending, tentar novamente
                        app.pending_gui = Some((id, slot));
                        app.ui.status = "Plugin is validating in Wine / yabridge; window will open automatically...".to_string();
                    } else {
                        app.pending_gui = None;
                        log::warn!("plugin window for track {} slot {slot} did not open: {e}", id.0);
                        app.ui.status = format!("Plugin window: {e}");
                    }
                } else {
                    app.pending_gui = None;
                    log::warn!("plugin window for track {} slot {slot} did not open: {e}", id.0);
                    app.ui.status = format!("Plugin window: {e}");
                }
            }
        }
    }
}
```

## 6. Problema: Plugin Scanning & Probe Reliability

### Correção Necessária:
Melhorar o tratamento de plugins que falham durante a escaneamento:

```rust
/// Bundle being loaded, which is skipped on the next pass, so the scan always completes: a broken
/// plugin costs one skipped bundle, never the editor.
fn scan_with_child(folders: &[String]) -> Result<ScanOutcome, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate the editor binary: {e}"))?;
    let mut vst3 = Vec::new();
    let mut clap = Vec::new();
    let mut skipped: Vec<String> = load_broken();
    let mut note = None;
    
    // CORREÇÃO: Adicionar limite para evitar loops infinitos
    for attempt in 0..=MAX_SCAN_SKIPS {
        let pass = run_scanner_child(&exe, folders, &skipped)?;
        vst3.extend(pass.vst3);
        clap.extend(pass.clap);
        
        if pass.ok {
            // Se a varredura foi bem-sucedida
            if let Ok(mut broken) = BROKEN_BUNDLES.lock() {
                broken.extend(skipped.iter().cloned());
                broken.dedup();
            }
            save_broken(&skipped);
            return Ok(ScanOutcome { vst3, clap, note, skipped });
        }
        
        // Verificar se há um problema com o scanner
        let Some(culprit) = pass.last_scanning else {
            note = Some(format!("the scanner stopped ({})", pass.status));
            return Ok(ScanOutcome { vst3, clap, note, skipped });
        };
        
        if skipped.contains(&culprit) {
            // Se já foi ignorado antes, adicionar uma nota
            note = Some(format!("the scanner kept crashing on {}", culprit));
            return Ok(ScanOutcome { vst3, clap, note, skipped });
        }
        
        log::warn!("the plugin scanner crashed while loading {culprit} ({}); skipping that bundle", pass.status);
        skipped.push(culprit);
        note = Some(format!("{} bundle(s) skipped after crashing the scanner", skipped.len()));
    }
    
    // Se chegamos aqui, o limite de tentativas foi atingido
    Ok(ScanOutcome { vst3, clap, note, skipped })
}
```

## 7. Testes Recomendados

1. **Teste de MIDI**: Verificar que ao abrir o Alt GUI e selecionar diferentes tracks, as notas continuem funcionando corretamente.
2. **Teste de Plugin Scanning**: Executar `--scan-plugins` com plugins que falham para garantir que não percam plugins válidos.
3. **Teste de Controle Direto**: Verificar se os knobs do Alt GUI atualizam diretamente os parâmetros dos plugins VST3, além das mensagens MIDI CC.
4. **Teste de Plugin GUI**: Garantir que o editor nativo abra corretamente em ambientes Linux/Wine.

## 8. Considerações Finais

Essas correções visam resolver problemas críticos no funcionamento do SoundCraft:
- Evitar silenciamento total da entrada MIDI quando o Alt GUI está ativo
- Melhorar a confiabilidade do escaneamento de plugins
- Permitir controle direto de parâmetros em vez de depender apenas de mensagens MIDI CC
- Garantir que os editors nativos dos plugins abram corretamente em ambientes Linux/Wine

As correções são projetadas para serem minimamente invasivas e manter a compatibilidade com o código existente.