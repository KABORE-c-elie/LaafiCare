// Petit serveur local pour essayer la construction de production comme elle
// sera servie : en-têtes de sécurité (CSP stricte), compression brotli, cache,
// et transmission de /api au backend. Modules de Node seulement (http, fs,
// path, zlib) : aucune dépendance. Ce n'est PAS un serveur de production.
//
// Lancement, depuis web/ : npm run build, puis
//   node outils/serveur-production.mjs munaseb          → http://127.0.0.1:8082
//   node outils/serveur-production.mjs administration   → http://127.0.0.1:8083
// (8080 : backend ; 8081 : Adminer.)

import { createServer, request } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { extname, join, normalize, resolve, sep } from 'node:path';
import { brotliCompressSync, constants } from 'node:zlib';

// Une application par port : chacune sera déployée à part, à sa propre adresse.
const APPLICATIONS = { munaseb: 8082, administration: 8083 };
const APPLICATION = process.argv[2] ?? 'munaseb';
if (!Object.hasOwn(APPLICATIONS, APPLICATION)) {
  console.error(`Application inconnue : ${APPLICATION}. Choix : ${Object.keys(APPLICATIONS).join(', ')}.`);
  process.exit(1);
}
const PORT = APPLICATIONS[APPLICATION];
const RACINE = resolve(import.meta.dirname, '..', 'dist', APPLICATION, 'browser');
const BACKEND = { hote: '127.0.0.1', port: 8080 };

// CSP demandée par le porteur, sans assouplissement. Les Trusted Types
// (`require-trusted-types-for 'script'`) ne sont PAS activés : essayés le
// 2026-10-20, ils bloquent l'enregistrement du service worker d'Angular
// (`navigator.serviceWorker.register` reçoit une chaîne, pas un
// TrustedScriptURL ; console : « This document requires 'TrustedScriptURL'
// assignment », puis NG05604). Les activer demanderait du code (voir docs/decisions.md).
const CSP = [
  "default-src 'self'",
  "script-src 'self'",
  "style-src 'self'",
  "connect-src 'self'",
  "img-src 'self' data:",
  "object-src 'none'",
  "base-uri 'none'",
  "frame-ancestors 'none'",
  "form-action 'self'",
].join('; ');

const EN_TETES_SECURITE = {
  'Content-Security-Policy': CSP,
  'X-Content-Type-Options': 'nosniff',
  'Referrer-Policy': 'no-referrer',
};

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.json': 'application/json; charset=utf-8',
  '.txt': 'text/plain; charset=utf-8',
};

// Fichiers dont le nom contient une empreinte (outputHashing) : jamais modifiés,
// cache d'un an (web.dev, « Prevent unnecessary network requests with the HTTP Cache »).
const AVEC_EMPREINTE = /-[A-Za-z0-9_-]{8}\.(js|css)$/;

const compresses = new Map();

async function servirFichier(req, res, chemin) {
  const contenu = await readFile(chemin);
  const type = TYPES[extname(chemin)] ?? 'application/octet-stream';
  const enTetes = {
    ...EN_TETES_SECURITE,
    'Content-Type': type,
    'Cache-Control': AVEC_EMPREINTE.test(chemin) ? 'public, max-age=31536000, immutable' : 'no-cache',
    Vary: 'Accept-Encoding',
  };
  const accepteBrotli = /\bbr\b/.test(req.headers['accept-encoding'] ?? '');
  if (accepteBrotli) {
    let corps = compresses.get(chemin);
    if (corps === undefined) {
      corps = brotliCompressSync(contenu, { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } });
      compresses.set(chemin, corps);
    }
    res.writeHead(200, { ...enTetes, 'Content-Encoding': 'br', 'Content-Length': corps.length });
    res.end(req.method === 'HEAD' ? undefined : corps);
    return;
  }
  res.writeHead(200, { ...enTetes, 'Content-Length': contenu.length });
  res.end(req.method === 'HEAD' ? undefined : contenu);
}

function transmettre(req, res) {
  const vers = request(
    {
      host: BACKEND.hote,
      port: BACKEND.port,
      method: req.method,
      path: req.url,
      headers: { ...req.headers, host: `${BACKEND.hote}:${BACKEND.port}` },
    },
    (reponse) => {
      res.writeHead(reponse.statusCode ?? 502, { ...reponse.headers, ...EN_TETES_SECURITE });
      reponse.pipe(res);
    },
  );
  vers.on('error', () => {
    res.writeHead(502, { ...EN_TETES_SECURITE, 'Content-Type': 'text/plain; charset=utf-8' });
    res.end('Backend injoignable');
  });
  req.pipe(vers);
}

const serveur = createServer(async (req, res) => {
  const url = new URL(req.url ?? '/', `http://127.0.0.1:${PORT}`);
  if (url.pathname.startsWith('/api/')) {
    transmettre(req, res);
    return;
  }
  if (req.method !== 'GET' && req.method !== 'HEAD') {
    res.writeHead(405, EN_TETES_SECURITE).end();
    return;
  }
  // Chemin ramené sous RACINE : aucune sortie du dossier par « .. ».
  const demande = normalize(decodeURIComponent(url.pathname)).replace(/^([/\\])+/, '');
  const chemin = join(RACINE, demande);
  if (chemin !== RACINE && !chemin.startsWith(RACINE + sep)) {
    res.writeHead(400, EN_TETES_SECURITE).end();
    return;
  }
  try {
    const infos = await stat(chemin);
    if (infos.isFile()) {
      await servirFichier(req, res, chemin);
      return;
    }
  } catch {
    // Absent : voir plus bas.
  }
  // Navigation de l'application (pas d'extension) : index.html ; sinon 404.
  if (extname(demande) === '') {
    await servirFichier(req, res, join(RACINE, 'index.html'));
    return;
  }
  res.writeHead(404, { ...EN_TETES_SECURITE, 'Content-Type': 'text/plain; charset=utf-8' }).end('Introuvable');
});

// 127.0.0.1 seulement : jamais exposé au réseau local. C'est une origine
// « potentially trustworthy » (W3C Secure Contexts, §3.1), où le service worker s'enregistre.
serveur.listen(PORT, '127.0.0.1', () => {
  console.log(`Application ${APPLICATION} (production) servie sur http://127.0.0.1:${PORT}`);
});
