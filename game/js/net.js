// Shared by the login screen and the multiplayer client: where the game server is, and the one avatar factory (so model files are parsed once).
import { AvatarFactory } from './avatars.js';

export const serverUrl = () => new URLSearchParams(location.search).get('server') || new URL('ws', location.href.replace(/^http/, 'ws')).href;   // same origin, next to the page: serve.js proxies it to the server
export const factory = new AvatarFactory();
