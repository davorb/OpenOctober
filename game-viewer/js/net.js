// Shared by the login screen and the multiplayer client: where the game server is, and the one avatar factory (so model files are parsed once).
import { AvatarFactory } from './avatars.js';

export const serverUrl = () => new URLSearchParams(location.search).get('server') || `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.hostname || 'localhost'}:1338`;
export const factory = new AvatarFactory();
