#!/usr/bin/env bash
set -euo pipefail
sha256sum --check /bootstrap/data.sha256
exec psql --no-psqlrc --set ON_ERROR_STOP=1 --file /bootstrap/seed.sql
