#!/usr/bin/env python3
"""Serve the viewer at / and the converted game data at /data/.
Usage: python3 serve.py [port]"""
import http.server, os, sys
HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.normpath(os.path.join(HERE, '..', 'converted'))

class H(http.server.SimpleHTTPRequestHandler):
    def translate_path(self, path):
        p = http.server.SimpleHTTPRequestHandler.translate_path(self, path)
        data_prefix = os.path.join(os.getcwd(), 'data')
        if p == data_prefix or p.startswith(data_prefix + os.sep):
            return os.path.join(DATA, os.path.relpath(p, data_prefix))
        return p
    def end_headers(self):
        self.send_header('Cache-Control', 'no-cache' if not self.path.startswith('/data/') else 'max-age=3600')
        super().end_headers()
    def log_message(self, *a): pass

if __name__ == '__main__':
    os.chdir(HERE)
    if not os.path.exists('catalog.json'):
        import build_catalog; build_catalog.build()
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8000
    print(f'OpenOctober viewer: http://localhost:{port}/')
    http.server.ThreadingHTTPServer(('', port), H).serve_forever()
