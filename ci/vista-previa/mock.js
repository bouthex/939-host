/* Backend simulado para ver la interfaz sin Tauri (solo vista previa y pruebas de pantalla).
   Las formas de los datos salen de datos.json (generado con las estructuras reales de Rust). */
(function () {
  const D = window.__DATOS__;
  const cfg = D.config;
  const now = new Date().toISOString();
  const server = (over = {}) => Object.assign({ config: cfg, logoUrl: null, status: 'running', players: ['Ana', 'Beto'], startedAt: now, memoryUsedMb: 1830, mspt: 12.4, msptP95: 18, lastError: null, needsRestart: false, installing: false, worldSizeMb: 412, paused: false, saving: false, restarting: false, phase: null, countdown: null, pauseSupported: true, reach: 'ok', reachInfo: null, reachError: null, joinIssue: null, modsCount: 12 }, over);
  const servers = [server({ logoUrl: 'logo-939.png', config: Object.assign({}, cfg, { logo: '939' }) }), server({ config: Object.assign({}, cfg, { id: 'srv-2', name: 'Creativo', loader: 'vanilla', loaderVersion: null }), status: 'stopped', players: [] })];
  const friends = { available: true, friends: [
    { node: 'n1', name: 'Archimak', online: true, playing: 'Supervivencia con amigos', available: true, unread: 2, avatar: null, sharedGroups: ['Supervivencia con amigos'] },
    { node: 'n2', name: 'Kaki', online: false, playing: null, available: false, unread: 0, avatar: null, sharedGroups: [] }],
    requests: [{ node: 'n3', name: 'Lucía' }], notices: [], blocked: 0 };
  const T = {
    app_info: { version: '0.9.0', dataDir: 'C:\\Users\\usuario\\AppData\\Local\\939Host', node: { nodeId: 'abc', shortId: 'ABC' }, settings: D.settings },
    list_servers: servers, get_server: servers[0], friends_view: friends, trash_list: [],
    friend_chat: [{ fromMe: false, text: '¿Jugamos hoy?', at: now, delivered: true }, { fromMe: true, text: 'Dale, a las 21', at: now, delivered: true }],
    share_status: { enabled: false, group: null, invites: [], routePort: null, relays: true, nodeId: 'abc', privateMode: false, recoverySet: false, recoveryPending: false, successor: null, succession: null, pendingMods: [], trustCohostMods: false }, update_status: { enabled: false, channel: 'estable', current: '0.9.0', status: 'desactivado', message: 'Las actualizaciones automáticas todavía no están configuradas por el proyecto. Las versiones nuevas se instalan a mano.' },
    hardware_snapshot: { cpuModel: 'Procesador de prueba', physicalCores: 8, logicalCores: 16, cpuMhz: 3600, ramTotalMb: 16384, ramAvailableMb: 9000, os: 'Windows 11', hostName: 'PC', disks: [{ name: 'C:', totalGb: 476, freeGb: 210 }], benchmark: null, assessment: { rating: 'Buena para alojar', reasons: ['16 GB de memoria'], recommendedServerRamMb: 4096, maxServerRamMb: 8192 } },
    hardware_live: { cpuPercent: 12, ramAvailableMb: 9000 },
    list_runtimes: [], save_settings: D.settings, client_profiles: [], remote_list: [], hosting_servers: [], host_requests_pending: [], last_close_notes: [], profile_photo_get: {},
    mods_list: [], snapshots_list: { items: [
      { id: 's1', createdAt: now, label: 'Automático', kind: 'auto', mcVersion: '1.21.4', fileCount: 1200, totalBytes: 420e6, newBytes: 3e6, pauseMs: 40, epoch: 1, seq: 4 },
      { id: 's2', createdAt: now, label: 'Antes de actualizar mods', kind: 'manual', mcVersion: '1.21.4', fileCount: 1190, totalBytes: 418e6, newBytes: 9e6, pauseMs: 35, epoch: 1, seq: 3 }], stats: { count: 2, bytes: 430e6 } },
    storage_usage: { snapshotsBytes: 430e6, backupsBytes: 0, capBytes: 20 * 1073741824, protected: [] },
    world_info: { exists: true, levelName: 'world', seed: '-4172144997902289642', sizeBytes: 412e6, world: 'world', dimensions: [{ name: 'Mundo normal', sizeBytes: 300e6 }, { name: 'Nether', sizeBytes: 80e6 }, { name: 'End', sizeBytes: 32e6 }], playersData: 3 },
    files_list: { entries: [{ name: 'world', path: 'world', dir: true, size: 0, modified: now, kind: 'world', locked: true }, { name: 'mods', path: 'mods', dir: true, size: 0, modified: now, kind: 'mods', locked: false }, { name: 'server.properties', path: 'server.properties', dir: false, size: 1300, modified: now, kind: 'config', locked: false }], remote: false },
    public_address_get: '',
    alias_status: { items: [], suggestion: { 'srv-demo': 'supervivencia' } }, list_backups: [], players_list: [], modrinth_modpacks: { hits: [], total: 0 },
  };
  const OBJ = new Set(['get_server', 'world_info', 'files_list', 'snapshots_list', 'alias_status', 'app_info', 'share_status', 'storage_usage', 'game_assets_status', 'modrinth_modpacks', 'cf_status', 'alias_status', 'profile_photo_get']);
  const NUL = new Set(['data_notice', 'start_server', 'stop_server', 'server_logo_default', 'server_logo_set', 'server_logo_clear']);
  const listeners = {};
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => { (window.__INVOCADOS__ = window.__INVOCADOS__ || []).push(cmd); if (cmd in T) return JSON.parse(JSON.stringify(typeof T[cmd] === 'function' ? T[cmd](args) : T[cmd])); if (NUL.has(cmd)) return null; return OBJ.has(cmd) ? {} : []; } },
    event: { listen: async (ev, fn) => { (listeners[ev] = listeners[ev] || []).push(fn); return () => {}; }, emit: (ev, p) => (listeners[ev] || []).forEach((f) => f({ payload: p })) },
    dialog: { open: async () => null, save: async () => null },
    window: { getCurrentWindow: () => ({ onDragDropEvent: async () => () => {} }) },
    webview: { getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }) },
  };
  window.__MOCK__ = { T, emit: window.__TAURI__.event.emit, server, servers };
})();
