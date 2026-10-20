let THREE = null;
const threePromise = import('https://cdn.jsdelivr.net/npm/three@0.165.0/build/three.module.js').catch((error) => { console.error('Three.js failed to load:', error); return null; });

const $ = (id) => document.getElementById(id);
const screenName = $('name-screen');
const screenLobby = $('lobby-screen');
const screenGame = $('game-screen');
let guestName = localStorage.getItem('egb_guest_name') || '';
let currentSession = null;
let currentRoomId = null;
let currentRoomName = '';
let socket = null;
let latestState = null;
let selectedUnit = 'vanguard';
let selectedLevel = 1;
let pendingJoinRoom = null;
let toastTimer = null;
let roomRefreshTimer = null;
let scene, camera, renderer, ground, raycaster, pointer;
let animFrame = null;
const unitObjects = new Map();
const buildingObjects = new Map();
const pointObjects = new Map();
const materials = {};
const TEAM = { blue: 0x3988d4, red: 0xd64d44 };
const SKIN = 0xf0c8a0;
const WORLD_W = 120;
const WORLD_D = 80;

function showScreen(which) {
  screenName.classList.toggle('hidden', which !== 'name');
  screenLobby.classList.toggle('hidden', which !== 'lobby');
  screenGame.classList.toggle('hidden', which !== 'game');
}
function notify(message, isError = false) {
  const node = $('global-toast');
  node.textContent = message;
  node.style.background = isError ? '#2b1a18' : '#172c1e';
  node.style.borderColor = isError ? 'rgba(237,119,109,.45)' : 'rgba(237,201,120,.38)';
  node.style.color = isError ? '#ffb1a7' : '#f0d99c';
  node.classList.remove('hidden');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => node.classList.add('hidden'), 3200);
}
function showBattleToast(message) {
  const node = $('toast');
  node.textContent = message;
  node.classList.remove('hidden');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => node.classList.add('hidden'), 2300);
}
async function api(path, options = {}) {
  const response = await fetch(path, {
    ...options,
    headers: { 'Content-Type': 'application/json', ...(options.headers || {}) },
  });
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.error || `Lỗi máy chủ (${response.status})`);
  return data;
}

// Guest identity is only a display name stored in this browser; there is no account system.
$('guest-name').value = guestName;
$('name-form').addEventListener('submit', (event) => {
  event.preventDefault();
  const value = $('guest-name').value.trim();
  if (value.length < 2 || value.length > 18) return notify('Tên phải dài từ 2 đến 18 ký tự.', true);
  guestName = value;
  localStorage.setItem('egb_guest_name', guestName);
  $('lobby-player-name').textContent = guestName;
  showScreen('lobby');
  refreshRooms();
  startRoomRefresh();
});
$('change-name').addEventListener('click', () => {
  $('guest-name').value = guestName;
  showScreen('name');
});

function startRoomRefresh() {
  clearInterval(roomRefreshTimer);
  roomRefreshTimer = setInterval(() => { if (!screenLobby.classList.contains('hidden')) refreshRooms(); }, 4500);
}
async function refreshRooms() {
  try {
    const rooms = await api('/api/rooms');
    renderRoomList(rooms);
  } catch (error) {
    $('room-list').innerHTML = `<tr><td colspan="4" class="empty-state">Không thể kết nối máy chủ.<br><button id="retry-rooms" class="join-button">Thử lại</button></td></tr>`;
    $('retry-rooms')?.addEventListener('click', refreshRooms);
    $('room-count').textContent = '—';
  }
}
function renderRoomList(rooms) {
  const query = $('room-search').value.trim().toLocaleLowerCase('vi');
  const filtered = rooms.filter((room) => room.name.toLocaleLowerCase('vi').includes(query));
  $('room-count').textContent = String(filtered.length);
  $('lobby-empty').classList.toggle('hidden', filtered.length > 0);
  $('room-list').innerHTML = '';
  if (!filtered.length) {
    const row = document.createElement('tr');
    row.innerHTML = `<td colspan="4" class="empty-state">${rooms.length ? 'Không tìm thấy phòng khớp với từ khóa.' : 'Chưa có phòng nào đang mở.'}</td>`;
    $('room-list').appendChild(row);
    return;
  }
  for (const room of filtered) {
    const tr = document.createElement('tr');
    const lock = room.has_password ? '<span class="lock-icon" title="Có mật khẩu">◆</span>' : '';
    tr.innerHTML = `
      <td><div class="room-name-cell"><span class="room-icon">♜</span><span>${escapeHtml(room.name)} ${lock}<small>Phòng 1v1 · ${room.has_password ? 'Riêng tư' : 'Công khai'}</small></span></div></td>
      <td>${escapeHtml(room.host || 'Khách')}</td>
      <td><span class="players-pill">${room.players}/2 người</span></td>
      <td><button class="join-button" data-room="${escapeHtml(room.id)}" data-room-name="${escapeHtml(room.name)}">Vào phòng ↗</button></td>`;
    $('room-list').appendChild(tr);
  }
  $('room-list').querySelectorAll('[data-room]').forEach((button) => button.addEventListener('click', () => {
    pendingJoinRoom = { id: button.dataset.room, name: button.dataset.roomName };
    $('join-modal-title').textContent = pendingJoinRoom.name;
    $('join-password').value = '';
    $('join-error').textContent = '';
    $('join-modal').classList.remove('hidden');
    $('join-password').focus();
  }));
}
function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (ch) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[ch]);
}
$('room-search').addEventListener('input', refreshRooms);
$('refresh-rooms').addEventListener('click', refreshRooms);
let savedRoomName = null;
let savedRoomPassword = null;
$('room-name').addEventListener('input', () => { savedRoomName = null; $('room-name-feedback').textContent = ''; $('room-name-feedback').classList.remove('error'); });
$('room-password').addEventListener('input', () => { savedRoomPassword = null; $('room-password-feedback').textContent = ''; });
$('save-room-name').addEventListener('click', async () => {
  $('room-name-feedback').classList.remove('error');
  const name = $('room-name').value.trim();
  try {
    await api('/api/rooms/check-name', { method: 'POST', body: JSON.stringify({ name }) });
    savedRoomName = name;
    $('room-name-feedback').textContent = 'Tên phòng đã được lưu và chưa bị trùng.';
  } catch (error) {
    savedRoomName = null;
    $('room-name-feedback').textContent = error.message;
    $('room-name-feedback').classList.add('error');
  }
});
$('save-room-password').addEventListener('click', () => {
  savedRoomPassword = $('room-password').value;
  $('room-password-feedback').textContent = savedRoomPassword ? 'Mật khẩu đã được lưu cho phòng sắp tạo.' : 'Đã lưu lựa chọn: phòng công khai, không mật khẩu.';
});
$('create-room-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  $('create-error').textContent = '';
  const roomName = $('room-name').value.trim();
  const password = $('room-password').value;
  if (savedRoomName !== roomName) {
    $('create-error').textContent = 'Hãy nhấn “Lưu tên” và kiểm tra tên phòng trước khi tạo.';
    return;
  }
  if (savedRoomPassword !== password) {
    $('create-error').textContent = 'Hãy nhấn “Lưu pass” để xác nhận mật khẩu phòng.';
    return;
  }
  try {
    const result = await api('/api/rooms', { method: 'POST', body: JSON.stringify({ name: roomName, password, host_name: guestName }) });
    $('create-room-form').reset();
    savedRoomName = null; savedRoomPassword = null;
    $('room-name-feedback').textContent = ''; $('room-password-feedback').textContent = '';
    await enterRoom(result, roomName);
  } catch (error) {
    $('create-error').textContent = error.message;
  }
});
$('join-room-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  if (!pendingJoinRoom) return;
  $('join-error').textContent = '';
  try {
    const result = await api(`/api/rooms/${encodeURIComponent(pendingJoinRoom.id)}/join`, {
      method: 'POST', body: JSON.stringify({ name: guestName, password: $('join-password').value }),
    });
    const roomName = pendingJoinRoom.name;
    $('join-modal').classList.add('hidden');
    await enterRoom(result, roomName);
  } catch (error) { $('join-error').textContent = error.message; }
});
$('close-join-modal').addEventListener('click', () => $('join-modal').classList.add('hidden'));
$('join-modal').addEventListener('click', (event) => { if (event.target === $('join-modal')) $('join-modal').classList.add('hidden'); });

async function enterRoom(entered, roomName) {
  currentSession = entered;
  currentRoomId = entered.room_id;
  currentRoomName = roomName;
  latestState = null;
  $('battle-room-name').textContent = roomName;
  $('waiting-overlay').classList.remove('hidden');
  $('winner-overlay').classList.add('hidden');
  showScreen('game');
  initScene();
  connectSocket();
  $('map-own-label').textContent = `${entered.color.toUpperCase()} · YOUR SIDE`;
  window.setTimeout(resizeRenderer, 50);
}

function connectSocket() {
  if (socket) { try { socket.close(); } catch {} }
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  socket = new WebSocket(`${protocol}//${window.location.host}/ws/${encodeURIComponent(currentSession.room_id)}?player_id=${encodeURIComponent(currentSession.player_id)}&token=${encodeURIComponent(currentSession.token)}`);
  socket.addEventListener('open', () => showBattleToast('Đã kết nối máy chủ chiến trường.'));
  socket.addEventListener('message', (event) => {
    let message;
    try { message = JSON.parse(event.data); } catch { return; }
    if (message.type === 'state') updateGameState(message.state);
  });
  socket.addEventListener('close', () => {
    if (!screenGame.classList.contains('hidden') && currentSession) notify('Mất kết nối. Thử tải lại trang để kết nối lại.', true);
  });
  socket.addEventListener('error', () => notify('Không thể mở WebSocket. Hãy kiểm tra máy chủ.', true));
}
function send(message) {
  if (socket && socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify(message));
  else notify('Đang kết nối máy chủ, hãy thử lại sau ít giây.', true);
}

function updateGameState(state) {
  const previousState = latestState;
  if (previousState && previousState.players.length === 2 && state.players.length === 1) {
    for (const [, group] of buildingObjects) { scene?.remove(group); disposeGroup(group); }
    buildingObjects.clear();
  }
  latestState = state;
  const own = state.players.find((p) => p.id === state.you_id);
  const enemy = state.players.find((p) => p.id !== state.you_id);
  if (own) {
    $('own-player-name').textContent = own.name;
    $('own-gold').textContent = own.gold;
    $('own-label').textContent = `${own.color.toUpperCase()} · PHE CỦA BẠN`;
    $('map-own-label').textContent = `${own.color.toUpperCase()} · PHE CỦA BẠN`;
    $('mine-count').textContent = `${state.buildings.filter((b) => b.owner === own.side && b.kind === 'mine').length}/5`;
    const ownMine = state.buildings.filter((b) => b.owner === own.side && b.kind === 'mine');
    const nextLevel = ownMine.length ? ownMine.reduce((minimum, mine) => Math.min(minimum, mine.level), 5) : 1;
    $('mine-upgrade-cost').textContent = nextLevel >= 5 ? 'MAX CẤP' : `◈ ${nextLevel * 8}`;
    $('upgrade-mine').disabled = nextLevel >= 5 || ownMine.length === 0;
    $('upgrade-mine').style.opacity = $('upgrade-mine').disabled ? '.5' : '1';
    updateBuff(own);
    document.querySelectorAll('[data-command]').forEach((button) => button.classList.toggle('active', button.dataset.command === own.command));
  }
  $('enemy-player-name').textContent = enemy ? enemy.name : 'Đang chờ…';
  $('enemy-gold').textContent = enemy ? enemy.gold : '—';
  $('match-time').textContent = formatTime(state.time);
  $('waiting-overlay').classList.toggle('hidden', state.players.length >= 2 && state.started);
  updateScene(state);
  if (state.winner !== null && state.winner !== undefined) {
    $('winner-overlay').classList.remove('hidden');
    const won = own && own.side === state.winner;
    $('winner-title').textContent = won ? 'CHIẾN THẮNG' : 'THẤT BẠI';
    $('winner-title').style.color = won ? '#edc978' : '#ed8b80';
    $('winner-copy').textContent = won ? 'Lâu đài đối thủ đã thất thủ. Hãy tận hưởng chiến thắng!' : 'Lâu đài của bạn đã bị phá hủy. Hãy tập hợp lại quân đội.';
  } else $('winner-overlay').classList.add('hidden');
}
function formatTime(seconds) {
  const n = Math.max(0, Math.floor(seconds || 0));
  return `${String(Math.floor(n / 60)).padStart(2, '0')}:${String(n % 60).padStart(2, '0')}`;
}
function updateBuff(own) {
  const label = own.buff || '';
  const active = Boolean(label);
  const box = $('buff-current');
  box.classList.toggle('empty', !active);
  box.innerHTML = active
    ? `<span>✦</span><div><strong>${escapeHtml(label)} đang hoạt động</strong><small>Hiệu ứng được áp dụng cho phe của bạn</small></div>`
    : '<span>✧</span><div><strong>Chưa có buff</strong><small>Thời lượng hiệu ứng khoảng 9 giây</small></div>';
  const cooldown = Math.ceil(own.buff_cooldown || 0);
  $('roll-buff').disabled = cooldown > 0;
  $('buff-cooldown').textContent = cooldown > 0 ? `CHỜ ${cooldown}S` : 'SẴN SÀNG';
}

// Fixed orthographic Three.js camera: no orbit controls, no panning, bounded battlefield.
async function initScene() {
  if (renderer) { resizeRenderer(); return; }
  if (!THREE) {
    THREE = await threePromise;
    if (!THREE) {
      $('scene-loading').innerHTML = '<strong>Không tải được bộ dựng 3D</strong><span>Hãy kiểm tra kết nối CDN rồi tải lại trang.</span>';
      return;
    }
  }
  const root = $('viewport');
  scene = new THREE.Scene();
  scene.background = new THREE.Color(0x78b35c);
  scene.fog = new THREE.Fog(0x87bd68, 145, 270);
  camera = new THREE.OrthographicCamera(-65, 65, 43, -43, 0.1, 260);
  camera.position.set(0, 180, 0.01);
  camera.up.set(0, 0, -1);
  camera.lookAt(0, 0, 0);
  renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false, powerPreference: 'high-performance' });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  renderer.setSize(root.clientWidth, root.clientHeight);
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFSoftShadowMap;
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.13;
  root.prepend(renderer.domElement);
  raycaster = new THREE.Raycaster();
  pointer = new THREE.Vector2();
  scene.add(new THREE.HemisphereLight(0xf3ffe5, 0x426443, 2.2));
  const sun = new THREE.DirectionalLight(0xfff3d3, 2.0);
  sun.position.set(-38, 65, 24); sun.castShadow = true; sun.shadow.mapSize.set(1024, 1024);
  sun.shadow.camera.left = -75; sun.shadow.camera.right = 75; sun.shadow.camera.top = 60; sun.shadow.camera.bottom = -60;
  scene.add(sun);
  buildTerrain();
  createStaticDecorations();
  root.addEventListener('pointerdown', onMapPointerDown);
  window.addEventListener('resize', resizeRenderer);
  $('scene-loading').classList.add('hidden');
  const animate = () => {
    animFrame = requestAnimationFrame(animate);
    renderer.render(scene, camera);
  };
  animate();
}
function resizeRenderer() {
  if (!renderer || !camera) return;
  const root = $('viewport');
  const width = Math.max(1, root.clientWidth), height = Math.max(1, root.clientHeight);
  const aspect = width / height;
  const viewHeight = Math.max(82, 120 / aspect);
  camera.left = -viewHeight * aspect / 2;
  camera.right = viewHeight * aspect / 2;
  camera.top = viewHeight / 2;
  camera.bottom = -viewHeight / 2;
  camera.updateProjectionMatrix();
  renderer.setSize(width, height);
}
function mat(color, roughness = 0.92, extra = {}) {
  const key = `${color}-${roughness}-${JSON.stringify(extra)}`;
  if (!materials[key]) materials[key] = new THREE.MeshStandardMaterial({ color, roughness, ...extra });
  return materials[key];
}
function makeMesh(geometry, material, x, y, z, parent, cast = true) {
  const mesh = new THREE.Mesh(geometry, material);
  mesh.position.set(x, y, z);
  mesh.castShadow = cast;
  mesh.receiveShadow = true;
  parent.add(mesh);
  return mesh;
}
function buildTerrain() {
  const terrain = makeMesh(new THREE.PlaneGeometry(WORLD_W, WORLD_D), mat(0x75ad57), 0, -0.12, 0, scene, false);
  terrain.rotation.x = -Math.PI / 2;
  ground = terrain;
  // Subtle rolling patches lend the flat tactical field a soft, cartoon meadow finish.
  const patches = new THREE.Group(); scene.add(patches);
  for (let i = 0; i < 90; i++) {
    const x = (Math.random() - .5) * 116, z = (Math.random() - .5) * 76;
    const radiusX = 1.2 + Math.random() * 4.5, radiusZ = .5 + Math.random() * 2.7;
    const patch = makeMesh(new THREE.CircleGeometry(1, 9), mat([0x80b962,0x6eaa51,0x84b963,0x73a955][i%4]), x, -0.095, z, patches, false);
    patch.rotation.x = -Math.PI/2; patch.scale.set(radiusX, radiusZ, 1);
  }
  const grassGeometry = new THREE.ConeGeometry(0.075, 0.18, 3);
  const grassInstanced = new THREE.InstancedMesh(grassGeometry, mat(0x4f8c43), 850);
  const dummy = new THREE.Object3D();
  for (let i = 0; i < 850; i++) {
    let x = (Math.random() - .5) * 116, z = (Math.random() - .5) * 76;
    if (Math.abs(z) < 3.5) z += z < 0 ? -4 : 4;
    dummy.position.set(x, -0.01, z); dummy.rotation.set(0, Math.random()*Math.PI, 0); dummy.scale.setScalar(.55 + Math.random()*.8); dummy.updateMatrix(); grassInstanced.setMatrixAt(i,dummy.matrix);
  }
  grassInstanced.receiveShadow = true; scene.add(grassInstanced);
  // Pale banks and a shallow blue-green river. Troops can cross it freely.
  const upperBank = makeMesh(new THREE.PlaneGeometry(WORLD_W, 7.5), mat(0xc5c184), 0, -0.075, 0, scene, false); upperBank.rotation.x=-Math.PI/2;
  // Reposition a second sand strip on both sides of the water, leaving the water channel visible.
  upperBank.position.z = 0;
  upperBank.scale.set(1,1,1);
  const bank1 = makeMesh(new THREE.PlaneGeometry(WORLD_W, 2.3), mat(0xd0c58b), 0, -0.04, 2.05, scene, false); bank1.rotation.x=-Math.PI/2;
  const bank2 = makeMesh(new THREE.PlaneGeometry(WORLD_W, 2.3), mat(0xd0c58b), 0, -0.04, -2.05, scene, false); bank2.rotation.x=-Math.PI/2;
  const river = makeMesh(new THREE.PlaneGeometry(WORLD_W, 2.2), new THREE.MeshStandardMaterial({color:0x56aeb5,roughness:.26,metalness:.03}), 0, 0.015, 0, scene, false); river.rotation.x=-Math.PI/2;
  for (let i=0;i<32;i++) {
    const x=(Math.random()-.5)*116, z=(Math.random()-.5)*1.55;
    const ripple=makeMesh(new THREE.PlaneGeometry(.6+Math.random()*1.7,.035),mat(0x9ce0db,.35,{transparent:true,opacity:.55}),x,.03,z,scene,false);
    ripple.rotation.x=-Math.PI/2; ripple.rotation.z=(Math.random()-.5)*.08;
  }
  // Thin natural boundary line keeps the camera inside the play area without a visible UI frame.
  const border = new THREE.LineSegments(new THREE.EdgesGeometry(new THREE.BoxGeometry(WORLD_W, .03, WORLD_D)), new THREE.LineBasicMaterial({color:0xc5d79a,transparent:true,opacity:.55}));
  border.position.y=.02; scene.add(border);
}
function createStaticDecorations() {
  // Sparse shrubs and trees on the distant edges; the center stays readable for tiny armies.
  for (let side of [-1,1]) {
    for (let i=0;i<22;i++) {
      const x=-57+i*(114/21)+(Math.random()-.5)*1.8;
      makeTree(x, side*(36+Math.random()*3), .65+Math.random()*.55);
    }
  }
  for (let i=0;i<12;i++) {
    const z=-30+Math.random()*60;
    makeTree((Math.random()<.5?-1:1)*(56+Math.random()*3),z,.5+Math.random()*.5);
  }
  // Two neutral standards mark the river objectives.
  for (const point of [{id:1,x:-17.4,z:0},{id:2,x:17.4,z:0}]) {
    const group = new THREE.Group(); group.position.set(point.x,0,point.z); scene.add(group);
    makeMesh(new THREE.CylinderGeometry(.48,.55,.14,9),mat(0x887b4c),0,.07,0,group);
    makeMesh(new THREE.CylinderGeometry(.06,.07,2.8,6),mat(0xe8dfbd),0,1.45,0,group);
    const flag=makeMesh(new THREE.BoxGeometry(.95,.55,.06),mat(0xead38b),.48,2.25,0,group); flag.rotation.y=.08;
    makeMesh(new THREE.SphereGeometry(.11,7,6),mat(0xffe3a1),0,2.91,0,group);
    pointObjects.set(point.id,group);
  }
}
function makeTree(x,z,scale=1) {
  const g=new THREE.Group();g.position.set(x,0,z);g.scale.setScalar(scale);scene.add(g);
  makeMesh(new THREE.CylinderGeometry(.18,.27,1.1,5),mat(0x76513a),0,.55,0,g);
  makeMesh(new THREE.ConeGeometry(1.05,2.1,7),mat(0x376e3a),0,1.85,0,g);
  makeMesh(new THREE.ConeGeometry(.78,1.7,7),mat(0x4f8c45),0,2.65,0,g);
  makeMesh(new THREE.ConeGeometry(.52,1.35,7),mat(0x6aa14f),0,3.28,0,g);
}
function teamHex(owner) {
  const player=latestState?.players?.find((p)=>p.side===owner);
  if (player) return TEAM[player.color] || TEAM.blue;
  const solePlayer=latestState?.players?.[0];
  if (solePlayer) return TEAM[solePlayer.color === 'blue' ? 'red' : 'blue'];
  return TEAM[owner===0?'blue':'red'];
}
function worldToScene(x,y) { return { x:(x/1000-.5)*WORLD_W, z:(y/1000-.5)*WORLD_D }; }
function createCastle(owner) {
  const group = new THREE.Group();
  const color = teamHex(owner);
  const stone=mat(0x9a9b83), lightStone=mat(0xb8b59a), roof=mat(color), darkRoof=mat(color===TEAM.blue?0x225b92:0x902e2a), wood=mat(0x634c34), banner=mat(color);
  makeMesh(new THREE.BoxGeometry(8.6,.7,7),stone,0,.5,0,group);
  makeMesh(new THREE.BoxGeometry(7.7,2.3,6.2),lightStone,0,1.75,0,group);
  // Corner towers plus central keep; all forms are low-poly and readable from directly above.
  for (const [x,z] of [[-3.25,-2.45],[3.25,-2.45],[-3.25,2.45],[3.25,2.45]]) {
    makeMesh(new THREE.CylinderGeometry(1.18,1.22,2.7,6),lightStone,x,1.75,z,group);
    makeMesh(new THREE.ConeGeometry(1.28,1.25,6),roof,x,3.6,z,group);
    makeMesh(new THREE.BoxGeometry(.15,.5,.15),mat(0xe7dcb2),x,4.45,z,group);
  }
  makeMesh(new THREE.BoxGeometry(3.2,3.2,3.1),stone,0,2.25,0,group);
  makeMesh(new THREE.ConeGeometry(2.15,1.8,4),darkRoof,0,4.55,0,group);
  makeMesh(new THREE.BoxGeometry(1.0,1.75,.12),wood,0,1.18,3.12,group);
  makeMesh(new THREE.BoxGeometry(2.8,.18,.13),banner,0,3.6,3.8,group);
  const hpBarGroup=new THREE.Group();hpBarGroup.position.set(0,5.0,0);group.add(hpBarGroup);
  const back=makeMesh(new THREE.BoxGeometry(8.5,.18,.2),mat(0x1a2519),0,0,0,hpBarGroup,false);
  const fill=makeMesh(new THREE.BoxGeometry(8.35,.19,.22),mat(0x74d17b),0,.01,.02,hpBarGroup,false);fill.userData.hpFill=true;
  group.userData.hpFill=fill;group.userData.hpBack=back;group.userData.baseScale=1;
  return group;
}
function createMine(building) {
  const group = new THREE.Group();
  makeMesh(new THREE.CylinderGeometry(1.2,1.6,.5,7),mat(0x897653),0,.24,0,group);
  makeMesh(new THREE.DodecahedronGeometry(.72,0),mat(0xc4a94e),-.35,.65,-.05,group);
  makeMesh(new THREE.DodecahedronGeometry(.66,0),mat(0xf0d36a),.36,.58,.08,group);
  makeMesh(new THREE.DodecahedronGeometry(.44,0),mat(0xdcb74f),.1,.95,-.05,group);
  const banner=makeMesh(new THREE.BoxGeometry(.75,.23,.05),mat(teamHex(building.owner)),.02,1.3,.35,group);banner.rotation.z=-.1;
  return group;
}
function createUnit(unit) {
  const group = new THREE.Group();
  const color = mat(teamHex(unit.owner));
  const dark = mat(teamHex(unit.owner)===TEAM.blue?0x21568b:0x922f2b);
  const metal = mat(0xb8c4bd,.42,{metalness:.18});
  const skin = mat(SKIN);
  const leather = mat(0x5c4934);
  const cloth = color;
  const body = () => {
    makeMesh(new THREE.SphereGeometry(.31,8,6),cloth,0,.67,0,group).scale.set(.9,1.22,.72);
    makeMesh(new THREE.SphereGeometry(.205,8,6),skin,0,1.12,0,group);
    makeMesh(new THREE.CylinderGeometry(.18,.28,.15,7),dark,0,.91,0,group);
    makeMesh(new THREE.BoxGeometry(.15,.43,.16),cloth,-.28,.65,0,group).rotation.z=-.15;
    makeMesh(new THREE.BoxGeometry(.15,.43,.16),cloth,.28,.65,0,group).rotation.z=.15;
    makeMesh(new THREE.CylinderGeometry(.09,.11,.42,5),dark,-.12,.27,0,group);
    makeMesh(new THREE.CylinderGeometry(.09,.11,.42,5),dark,.12,.27,0,group);
  };
  if (unit.kind === 'mine') return createMine({owner:unit.owner});
  if (unit.kind === 'catapult') {
    makeMesh(new THREE.BoxGeometry(1.4,.3,.8),leather,0,.65,0,group);
    for (const x of [-.48,.48]) for (const z of [-.42,.42]) makeMesh(new THREE.CylinderGeometry(.22,.22,.12,8),mat(0x4b3525),x,.28,z,group).rotation.x=Math.PI/2;
    makeMesh(new THREE.BoxGeometry(.17,1.45,.17),mat(0x927046),0,1.2,-.08,group).rotation.z=-.42;
    makeMesh(new THREE.SphereGeometry(.21,7,6),mat(0x6f6c61),-.42,1.58,-.08,group);
    makeMesh(new THREE.BoxGeometry(1.7,.12,.12),dark,0,1.45,0,group).rotation.z=-.42;
  } else if (unit.kind === 'elephant') {
    makeMesh(new THREE.SphereGeometry(.73,9,7),mat(0x8f9890),0,.75,0,group).scale.set(1.12,.82,.85);
    makeMesh(new THREE.SphereGeometry(.36,8,6),mat(0x9da79d),.56,.84,.02,group);
    makeMesh(new THREE.CylinderGeometry(.1,.14,.7,6),mat(0x9da79d),.83,.53,.08,group).rotation.z=-.25;
    for (const [x,z] of [[-.42,-.38],[-.42,.38],[.4,-.36],[.4,.36]]) makeMesh(new THREE.CylinderGeometry(.12,.15,.48,5),mat(0x747c73),x,.29,z,group);
    makeMesh(new THREE.BoxGeometry(.62,.16,.54),dark,-.05,1.38,0,group);
    makeMesh(new THREE.SphereGeometry(.19,8,6),skin,-.05,1.62,0,group);
    makeMesh(new THREE.CylinderGeometry(.08,.08,.8,5),metal,.25,1.72,0,group).rotation.z=-.8;
  } else if (unit.kind === 'cavalry' || unit.kind === 'general') {
    makeMesh(new THREE.SphereGeometry(.57,8,6),mat(0x76533c),0,.63,0,group).scale.set(1.35,.72,.73);
    makeMesh(new THREE.SphereGeometry(.28,8,6),mat(0x886249),.58,.85,0,group).scale.set(1.05,.82,.8);
    for (const [x,z] of [[-.42,-.3],[-.42,.3],[.36,-.3],[.36,.3]]) makeMesh(new THREE.CylinderGeometry(.075,.1,.48,5),mat(0x67452e),x,.27,z,group);
    makeMesh(new THREE.SphereGeometry(.27,8,6),cloth,-.02,1.17,0,group).scale.set(.82,1.2,.72);
    makeMesh(new THREE.SphereGeometry(.17,8,6),skin,.03,1.55,0,group);
    makeMesh(new THREE.BoxGeometry(.1,1.2,.1),metal,.31,1.8,0,group).rotation.z=-.35;
    if (unit.kind === 'general') {
      makeMesh(new THREE.ConeGeometry(.25,.42,4),mat(0xf3db85),-.08,1.95,0,group);
      makeMesh(new THREE.BoxGeometry(.52,.32,.07),cloth,.0,2.33,.05,group);
    }
  } else {
    body();
    if (unit.kind === 'archer') {
      makeMesh(new THREE.TorusGeometry(.38,.055,5,8,Math.PI*1.15),mat(0x8b633e),.34,.85,.05,group).rotation.y=Math.PI/2;
      makeMesh(new THREE.CylinderGeometry(.035,.035,.8,5),mat(0xd0bd84),.39,.8,.04,group).rotation.z=-.5;
    } else if (unit.kind === 'shield') {
      makeMesh(new THREE.BoxGeometry(.16,.76,.5),metal,-.43,.71,.03,group);
      makeMesh(new THREE.BoxGeometry(.1,.8,.08),dark,-.43,.71,.04,group);
    } else if (unit.kind === 'mage') {
      makeMesh(new THREE.ConeGeometry(.28,.54,6),mat(0x3e345b),0,1.52,0,group);
      makeMesh(new THREE.SphereGeometry(.18,7,6),mat(0xf2a45c),.35,.93,.05,group);
      makeMesh(new THREE.SphereGeometry(.11,7,6),mat(0xfce2a1),.35,.93,.05,group);
    } else {
      makeMesh(new THREE.CylinderGeometry(.045,.045,.85,5),metal,.38,.95,0,group).rotation.z=-.25;
      if (unit.kind === 'vanguard' || unit.kind === 'royal') makeMesh(new THREE.BoxGeometry(.2,.2,.2),mat(0xdac89b),.38,1.4,0,group);
    }
  }
  // Tiny oval ground shadow keeps the figures visually anchored to the grass.
  const shadow=makeMesh(new THREE.CircleGeometry(.55,12),new THREE.MeshBasicMaterial({color:0x1b3820,transparent:true,opacity:.22,depthWrite:false}),0,.035,0,group,false);
  shadow.rotation.x=-Math.PI/2;
  group.scale.setScalar(unit.kind==='elephant'?0.98:unit.kind==='catapult'?0.86:0.78);
  return group;
}
function updateScene(state) {
  if (!scene) return;
  const activeUnits = new Set();
  for (const unit of state.units) {
    activeUnits.add(unit.id);
    let group=unitObjects.get(unit.id);
    if (!group) { group=createUnit(unit); scene.add(group); unitObjects.set(unit.id,group); }
    const pos=worldToScene(unit.x,unit.y);
    group.position.set(pos.x,0,pos.z);
    if (unit.hp < unit.max_hp*.35) group.rotation.y += .012;
  }
  for (const [id,group] of unitObjects) if (!activeUnits.has(id)) { scene.remove(group); disposeGroup(group); unitObjects.delete(id); }

  const activeBuildings = new Set();
  for (const building of state.buildings) {
    activeBuildings.add(building.id);
    let group=buildingObjects.get(building.id);
    if (!group) {
      if (building.kind==='castle') group=createCastle(building.owner); else group=createMine(building);
      scene.add(group); buildingObjects.set(building.id,group);
    }
    const pos=worldToScene(building.x,building.y);
    group.position.set(pos.x,0,pos.z);
    if (building.kind==='castle' && group.userData.hpFill) {
      const ratio=Math.max(0,building.hp/building.max_hp);
      group.userData.hpFill.scale.x=ratio;
      group.userData.hpFill.position.x=-4.15*(1-ratio);
      group.userData.hpFill.material=mat(ratio>.5?0x74d17b:ratio>.25?0xe2bd5c:0xe56b60);
    }
    if (building.kind==='mine') group.scale.setScalar(0.75 + (building.level-1)*.08);
  }
  for (const [id,group] of buildingObjects) if (!activeBuildings.has(id)) { scene.remove(group); disposeGroup(group); buildingObjects.delete(id); }
  for (const point of state.points || []) {
    const group=pointObjects.get(point.id);
    if (!group) continue;
    const flag=group.children.find((child)=>child.geometry?.type==='BoxGeometry');
    if (flag && point.owner!==null) flag.material=mat(teamHex(point.owner));
    else if (flag) flag.material=mat(0xead38b);
  }
}
function disposeGroup(group) {
  group.traverse((node) => { if (node.isMesh && node.geometry && !Object.values(materials).includes(node.material)) { /* shared geometries/materials are intentionally retained for reuse */ } });
}
function onMapPointerDown(event) {
  if (!latestState || !currentSession || !renderer || event.button !== 0) return;
  if (!$('waiting-overlay').classList.contains('hidden') || !$('winner-overlay').classList.contains('hidden')) return;
  if (event.target.closest('button')) return;
  const rect=renderer.domElement.getBoundingClientRect();
  pointer.x=((event.clientX-rect.left)/rect.width)*2-1;
  pointer.y=-((event.clientY-rect.top)/rect.height)*2+1;
  raycaster.setFromCamera(pointer,camera);
  const hits=raycaster.intersectObject(ground,false);
  if (!hits.length) return;
  const point=hits[0].point;
  const x=Math.max(12,Math.min(988,((point.x/WORLD_W)+.5)*1000));
  const y=Math.max(12,Math.min(988,((point.z/WORLD_D)+.5)*1000));
  const own=latestState.players.find((p)=>p.id===latestState.you_id);
  if (!own) return;
  const ownHalf=own.side===0?y<472:y>528;
  if (!ownHalf) { showBattleToast('Chỉ có thể triển khai quân trong phần sân của bạn.'); return; }
  if (selectedUnit==='mine') {
    const mines=latestState.buildings.filter((b)=>b.owner===own.side&&b.kind==='mine');
    if (mines.length>=5) { showBattleToast('Bạn đã đạt giới hạn 5 mỏ vàng.'); return; }
    const nearby=latestState.buildings.some((b)=>Math.hypot(b.x-x,b.y-y)<48);
    if (nearby) { showBattleToast('Hãy đặt mỏ cách xa công trình khác.'); return; }
    send({type:'build_mine',x,y,level:selectedLevel});
    showBattleToast('Đã gửi lệnh xây mỏ vàng.');
  } else {
    const costs={vanguard:5,archer:7,shield:7,mage:11,cavalry:10,elephant:12,catapult:14,general:15};
    const actualCost=Math.ceil((costs[selectedUnit]||0)*(1+0.18*(selectedLevel-1)));
    if (own.gold<actualCost) { showBattleToast('Không đủ vàng để triển khai quân này.'); return; }
    send({type:'deploy',unit:selectedUnit,x,y,level:selectedLevel});
    showBattleToast(`Triển khai ${unitName(selectedUnit)} · cấp ${selectedLevel}`);
  }
}
function updateUnitCardCosts() {
  const base={vanguard:5,archer:7,shield:7,mage:11,cavalry:10,elephant:12,catapult:14,general:15,mine:0};
  document.querySelectorAll('.army-card').forEach((button)=>{
    const cost=base[button.dataset.unit] ?? 0;
    const display=button.querySelector('.unit-cost');
    if(display) display.innerHTML=`◈ ${Math.ceil(cost*(1+0.18*(selectedLevel-1)))}`;
  });
}
function unitName(kind) { return ({vanguard:'lính tiên phong',archer:'cung thủ',shield:'lính khiên',mage:'pháp sư',cavalry:'kỵ binh',elephant:'kỵ tượng giáo',catapult:'máy bắn đá',general:'đại tướng quân',mine:'mỏ vàng'})[kind]||kind; }

const unitTooltips = {
  vanguard:'Lính tiên phong · 5 vàng · HP 54 · ATK 4 · DEF 4 · SPEED 3 · ATK SPEED 5 · Cận chiến',
  archer:'Cung thủ · 7 vàng · HP 34 · ATK 8 · DEF 2 · SPEED 2 · ATK SPEED 7 · Tầm xa 105',
  shield:'Lính khiên · 7 vàng · HP 112 · ATK 2 · DEF 12 · SPEED 2 · ATK SPEED 3 · Cận chiến',
  mage:'Pháp sư · 11 vàng · HP 42 · ATK 7 · DEF 2 · SPEED 2 · ATK SPEED 5 · AOE và thiêu đốt 4 giây',
  cavalry:'Kỵ binh · 10 vàng · HP 70 · ATK 8 · DEF 5 · SPEED 5 · ATK SPEED 5 · Lao thẳng xuyên đội hình',
  elephant:'Kỵ tượng giáo · 12 vàng · HP 98 · ATK 9 · DEF 7 · SPEED 5 · ATK SPEED 6 · Tầm đánh 23',
  catapult:'Máy bắn đá · 14 vàng · HP 60 · ATK 15 · DEF 5 · SPEED 1 · ATK SPEED 2 · Sát thương AOE',
  general:'Đại tướng quân · 15 vàng · HP 205 · ATK 2 · DEF 6 · SPEED 0 · ATK SPEED 2 · Gọi 2 lính hoàng gia mỗi 3 giây',
  mine:'Mỏ vàng · Miễn phí · HP 125 · +1 vàng/giây mỗi cấp · Tối đa 5 mỏ, cấp tối đa 5',
};
document.querySelectorAll('.army-card').forEach((button) => { button.title = unitTooltips[button.dataset.unit] || ''; button.addEventListener('click', () => {
  selectedUnit=button.dataset.unit;
  document.querySelectorAll('.army-card').forEach((other)=>other.classList.toggle('selected',other===button));
  showBattleToast(selectedUnit==='mine'?'Chọn vị trí trên sân của bạn để đặt mỏ vàng.':`Đã chọn ${unitName(selectedUnit)} · cấp ${selectedLevel}.`);
}); });
document.querySelectorAll('.level-button').forEach((button) => button.addEventListener('click', () => {
  selectedLevel=Number(button.dataset.level);
  document.querySelectorAll('.level-button').forEach((other)=>other.classList.toggle('active',other===button));
  $('selected-level-title').textContent=`· Cấp ${selectedLevel}`;
  updateUnitCardCosts();
}));
document.querySelectorAll('[data-command]').forEach((button) => button.addEventListener('click', () => {
  send({type:'command',mode:button.dataset.command});
}));
$('roll-buff').addEventListener('click', () => send({type:'roll_buff'}));
$('upgrade-mine').addEventListener('click', () => { send({type:'upgrade_mine'}); showBattleToast('Đã gửi lệnh nâng cấp mỏ vàng.'); });
$('copy-room').addEventListener('click', async () => {
  const roomName=currentRoomName;
  try { await navigator.clipboard.writeText(roomName); showBattleToast(`Đã sao chép tên phòng: ${roomName}`); }
  catch { showBattleToast(`Tên phòng: ${roomName}`); }
});
$('leave-room').addEventListener('click', leaveRoom);
$('return-lobby').addEventListener('click', leaveRoom);
async function leaveRoom() {
  const leaving = currentSession;
  if (leaving) {
    try { await api(`/api/rooms/${encodeURIComponent(leaving.room_id)}/leave`, { method: 'POST', body: JSON.stringify({ player_id: leaving.player_id, token: leaving.token }) }); } catch {}
  }
  if (socket) { try { socket.close(); } catch {} socket=null; }
  currentSession=null; currentRoomId=null; latestState=null;
  $('winner-overlay').classList.add('hidden');
  $('waiting-overlay').classList.add('hidden');
  for (const [,group] of unitObjects) { scene?.remove(group); }
  for (const [,group] of buildingObjects) { scene?.remove(group); }
  unitObjects.clear(); buildingObjects.clear();
  showScreen('lobby');
  await refreshRooms();
}
window.addEventListener('keydown', (event) => {
  if (!screenGame.classList.contains('hidden') && ['1','2','3','4'].includes(event.key) && !['INPUT','TEXTAREA'].includes(document.activeElement.tagName)) {
    const modes={'1':'attack','2':'retreat','3':'defend','4':'occupy'};
    send({type:'command',mode:modes[event.key]});
  }
});

updateUnitCardCosts();
if (guestName) {
  $('lobby-player-name').textContent=guestName;
  showScreen('lobby');
  refreshRooms(); startRoomRefresh();
} else showScreen('name');
