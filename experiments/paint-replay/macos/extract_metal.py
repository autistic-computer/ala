import collections, hashlib, json, pathlib, statistics, sys
import xml.etree.ElementTree as E
p = pathlib.Path(__file__).resolve().parent
label = sys.argv[1]
toc = E.parse(p / f'{label}-toc.xml').getroot()
target = toc.find('./run/info/target/process')
pid = target.get('pid')
xml = p / f'{label}-tables.xml'
root = E.parse(xml).getroot()
ids = {e.get('id'): e for e in root.iter() if e.get('id')}
def resolve(e):
    return ids[e.get('ref')] if e is not None and e.get('ref') else e
def value(e):
    e = resolve(e)
    return e.text if e is not None else None
rows = []
for row in root.findall('.//row'):
    process = resolve(row.find('process'))
    if process is None or value(process.find('pid')) != pid or row.find('gpu-channel-name') is None:
        continue
    # The first duration is GPU execution. The later duration is latency and
    # must not replace it. The first command-buffer ID matches app submissions.
    rows.append({'start_ns': int(value(row.find('start-time'))), 'duration_ns': int(value(row.findall('duration')[0])), 'channel': value(row.find('gpu-channel-name')), 'gpu': value(row.find('metal-device-name')), 'command_buffer_id': value(row.findall('metal-command-buffer-id')[0])})
assert rows, 'No GPU intervals for target PID'
start = min(row['start_ns'] for row in rows)
buffers = collections.defaultdict(list)
for row in rows:
    if row['start_ns'] >= start + 1_000_000_000:
        buffers[row['command_buffer_id']].append(row)
samples = [{'command_buffer_id': key, 'start_ns': min(r['start_ns'] for r in group), 'end_ns': max(r['start_ns']+r['duration_ns'] for r in group), 'channels': sorted({r['channel'] for r in group}), 'intervals': group} for key,group in buffers.items()]
durations = sorted((row['end_ns']-row['start_ns'])/1e6 for row in samples)
assert durations and all(value >= 0 for value in durations)
result = {'source_commit': json.loads((p/'pr1.json').read_text())['metadata']['source_commit'], 'label':label, 'target_pid':int(pid), 'target_name':target.get('name'), 'target_arguments':target.get('arguments'), 'recording_duration_seconds':float(toc.findtext('./run/info/summary/duration')), 'recording_end_reason':toc.findtext('./run/info/summary/end-reason'), 'target_termination_reason':target.get('termination-reason'), 'gpu_devices':sorted({r['gpu'] for r in rows}), 'target_gpu_intervals_including_startup':len(rows), 'excluded_startup_seconds':1, 'command_buffers':len(samples), 'gpu_encoder_envelope_ms':{'median':statistics.median(durations),'p95':durations[round((len(durations)-1)*.95)]}, 'note':'Target PID only. Envelope is the earliest GPU channel start to latest channel end for each application command buffer, not queue latency, display presentation, summed overlapping stage time, energy or power. Short captures under automatic GPU clocks are diagnostic, not evidence for general GPU speedups.', 'source_xml_sha256':hashlib.sha256(xml.read_bytes()).hexdigest(), 'samples':samples}
(p / f'{label}.target.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({k:v for k,v in result.items() if k!='samples'},indent=2))
