#!/bin/bash
set -euo pipefail

rm -rf dist
mkdir -p dist/svelteKit
npx vite build

cat > dist/svelteKit/run.sh << 'SCRIPT'
#!/bin/bash
exec node index.js
SCRIPT
chmod +x dist/svelteKit/run.sh

echo '{"type":"module"}' > dist/svelteKit/package.json
