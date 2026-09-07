#!/usr/bin/env python3
"""Deploy an ARM64 native release through the service's private S3 bucket and SSM."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import tarfile
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--stack', default='silicon-browser-production')
parser.add_argument('--region', default='us-east-1')
parser.add_argument('--binary', type=Path, default=Path('target/aarch64-unknown-linux-gnu/release/silicon-browser-backend'))
args = parser.parse_args()


def aws(*arguments):
    return subprocess.check_output(['aws', '--region', args.region, *arguments], text=True)


stack = json.loads(aws('cloudformation', 'describe-stacks', '--stack-name', args.stack))['Stacks'][0]
outputs = {item['OutputKey']: item['OutputValue'] for item in stack['Outputs']}
files = [args.binary, *(Path(__file__).parent / name for name in [
    'install-native.sh', 'backup.py', 'silicon-browser.service',
    'silicon-browser-backup.service', 'silicon-browser-backup.timer'])]
digest = hashlib.sha256()
for path in files:
    digest.update(path.name.encode() + b'\0' + path.read_bytes())
release = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%d') + '-' + digest.hexdigest()[:16]
remote = '/opt/silicon-browser/releases/' + release
object_url = f"s3://{outputs['ArtifactBucket']}/releases/{release}.tar.gz"
with tempfile.TemporaryDirectory(prefix='silicon-browser-release-') as directory:
    archive = Path(directory) / (release + '.tar.gz')
    with tarfile.open(archive, 'w:gz') as tar:
        for path in files:
            tar.add(path, arcname=path.name)
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    aws('s3', 'cp', str(archive), object_url, '--sse', 'AES256', '--only-show-errors')
    quote = shlex.quote
    commands = ['set -euo pipefail', 'umask 077',
                f'install -d -m 0755 {quote(remote)}',
                f'aws s3 cp {quote(object_url)} {quote(remote + ".tar.gz")} --region {quote(args.region)} --only-show-errors',
                f'echo {quote(checksum + "  " + remote + ".tar.gz")} | sha256sum -c -',
                f'tar -xzf {quote(remote + ".tar.gz")} -C {quote(remote)}',
                f'bash {quote(remote + "/install-native.sh")} {quote(remote)} {quote(outputs["ArtifactBucket"])} {quote(outputs["RuntimeSecretArn"])} {quote(args.region)}']
    request = {'DocumentName': 'AWS-RunShellScript', 'InstanceIds': [outputs['InstanceId']],
               'Parameters': {'commands': commands}, 'TimeoutSeconds': 180,
               'Comment': 'Install native Silicon Browser release ' + release}
    request_path = Path(directory) / 'request.json'
    request_path.write_text(json.dumps(request))
    command_id = json.loads(aws('ssm', 'send-command', '--cli-input-json', 'file://' + str(request_path)))['Command']['CommandId']
print(json.dumps({'release': release, 'command_id': command_id}), flush=True)
for _ in range(90):
    time.sleep(2)
    result = json.loads(aws('ssm', 'get-command-invocation', '--command-id', command_id, '--instance-id', outputs['InstanceId']))
    if result['Status'] in ['Pending', 'InProgress', 'Delayed']:
        continue
    print(json.dumps({'status': result['Status'], 'stdout': result['StandardOutputContent'], 'stderr': result['StandardErrorContent']}))
    raise SystemExit(0 if result['Status'] == 'Success' else 1)
raise SystemExit('SSM command still running; inspect the command ID before retrying')
