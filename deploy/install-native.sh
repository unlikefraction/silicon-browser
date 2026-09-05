#!/usr/bin/env bash
# Run as root on the private EC2 host after downloading and verifying the release archive.
# Rollback restores executables and configuration; database migrations must stay backward compatible.
set -euo pipefail
umask 077
release_directory="${1:?absolute extracted release directory}"
artifact_bucket="${2:?private S3 artifact bucket}"
runtime_secret="${3:?Secrets Manager ARN}"
[[ "$release_directory" == /* && -d "$release_directory" ]]
exec 9>/var/lock/silicon-browser-deploy.lock
flock -n 9
test -x "$release_directory/silicon-browser-backend"
for required_file in backup.py silicon-browser.service silicon-browser-backup.service silicon-browser-backup.timer; do
  test -r "$release_directory/$required_file"
done
install -d -m 0700 /etc/silicon-browser
install -d -o silicon-browser -g silicon-browser -m 0700 /var/lib/silicon-browser
previous_release=$(readlink -f /opt/silicon-browser/current || true)
configuration_files=(
  /etc/silicon-browser/runtime.env
  /etc/silicon-browser/artifact-bucket
  /etc/systemd/system/silicon-browser.service
  /etc/systemd/system/silicon-browser-backup.service
  /etc/systemd/system/silicon-browser-backup.timer
)
rollback_directory=$(mktemp -d /etc/silicon-browser/deploy-rollback.XXXXXX)
secret_file=$(mktemp /etc/silicon-browser/runtime.XXXXXX)
deployment_started=0
deployment_healthy=0
cleanup() {
  exit_status=$?
  trap - EXIT
  set +e
  if [[ "$deployment_started" == 1 && "$deployment_healthy" != 1 ]]; then
    # Keep the previous binary paired with the exact credentials and unit files it used.
    systemctl stop silicon-browser.service >/dev/null 2>&1
    for target in "${configuration_files[@]}"; do
      rm -f -- "$target"
      if [[ -f "$rollback_directory/$(basename "$target")" ]]; then
        cp -p -- "$rollback_directory/$(basename "$target")" "$target"
      fi
    done
    if [[ -n "$previous_release" && -d "$previous_release" ]]; then
      ln -sfn "$previous_release" /opt/silicon-browser/current.rollback
      mv -Tf /opt/silicon-browser/current.rollback /opt/silicon-browser/current
      systemctl daemon-reload
      if ! systemctl restart silicon-browser.service; then
        echo 'Previous release restart failed; operator recovery is required' >&2
      fi
    else
      systemctl disable --now silicon-browser.service silicon-browser-backup.timer >/dev/null 2>&1
      rm -f /opt/silicon-browser/current
      systemctl daemon-reload
    fi
  fi
  rm -f -- "$secret_file" /etc/silicon-browser/runtime.env.new
  rm -rf -- "$rollback_directory"
  exit "$exit_status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
for target in "${configuration_files[@]}"; do
  if [[ -f "$target" ]]; then
    cp -p -- "$target" "$rollback_directory/$(basename "$target")"
  fi
done
aws secretsmanager get-secret-value --region us-east-1 --secret-id "$runtime_secret" --query SecretString --output text > "$secret_file"
deployment_started=1
python3 - "$secret_file" <<'PY'
import json, os, re, sys
data = json.load(open(sys.argv[1]))
assert data and isinstance(data, dict)
for key, value in data.items():
    assert re.fullmatch(r'[A-Z][A-Z0-9_]*', key)
    assert isinstance(value, str) and '\n' not in value and '\r' not in value and '\0' not in value
with open('/etc/silicon-browser/runtime.env.new', 'w') as output:
    os.chmod(output.name, 0o600)
    for key, value in data.items():
        escaped = value.replace('\\', '\\\\').replace('"', '\\"')
        output.write(f'{key}="{escaped}"\n')
os.replace('/etc/silicon-browser/runtime.env.new', '/etc/silicon-browser/runtime.env')
PY
printf '%s\n' "$artifact_bucket" > /etc/silicon-browser/artifact-bucket
install -m 0644 "$release_directory/silicon-browser.service" /etc/systemd/system/silicon-browser.service
install -m 0644 "$release_directory/silicon-browser-backup.service" /etc/systemd/system/silicon-browser-backup.service
install -m 0644 "$release_directory/silicon-browser-backup.timer" /etc/systemd/system/silicon-browser-backup.timer
ln -sfn "$release_directory" /opt/silicon-browser/current.next
mv -Tf /opt/silicon-browser/current.next /opt/silicon-browser/current
systemctl daemon-reload
systemctl enable silicon-browser.service silicon-browser-backup.timer
systemctl restart silicon-browser.service
for attempt in $(seq 1 30); do
  if curl --fail --silent http://127.0.0.1:8080/healthz >/dev/null; then
    systemctl start silicon-browser-backup.timer
    deployment_healthy=1
    echo 'Native API release is healthy'
    exit 0
  fi
  sleep 1
done
echo 'Release failed its local health check' >&2
exit 1
