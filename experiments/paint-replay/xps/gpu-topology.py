#!/usr/bin/env python3
"""Summarize complete Mesa ANV u_trace frames; tolerate an unfinished final frame.

Run only after profiling ends:
  python3 experiments/paint-replay/xps/gpu-topology.py --exclude-seconds 1 capture-gpu.json.gz ...

GPU begin/end intervals can overlap. Their sum is not GPU execution time.
"""
import argparse
import collections
import gzip
import hashlib
import json
from pathlib import Path
import statistics


def parse_complete_frames(path):
    opener = gzip.open if path.suffix == '.gz' else open
    with opener(path, 'rt') as source:
        text = source.read()
    decoder = json.JSONDecoder()
    offset = text.index('[') + 1
    frames = []
    while True:
        while offset < len(text) and text[offset] in ' \n\t\r,':
            offset += 1
        if offset >= len(text) or text[offset] == ']':
            break
        try:
            value, end = decoder.raw_decode(text, offset)
        except json.JSONDecodeError:
            break
        frames.append(value)
        offset = end
    return frames, len(text[offset:].strip())


def earliest_ns(frame):
    return min(int(event['time_ns']) for batch in frame['batches']
               for event in batch['events'])


def distribution(values):
    ordered = sorted(values)
    return {'median_ms': statistics.median(ordered),
            'p95_ms': ordered[int(.95 * (len(ordered) - 1))],
            'min_ms': ordered[0], 'max_ms': ordered[-1]}


def summarize(path, exclude_seconds):
    all_frames, tail_chars = parse_complete_frames(path)
    beginning = earliest_ns(all_frames[0])
    frames = [frame for frame in all_frames
              if earliest_ns(frame) > beginning + exclude_seconds * 1e9]
    event_counts = collections.Counter()
    signature_counts = collections.Counter()
    batches_per_frame = collections.Counter()
    draws_per_frame = collections.Counter()
    vertices_per_frame = collections.Counter()
    params = collections.defaultdict(collections.Counter)
    times = collections.defaultdict(list)
    for frame in frames:
        signature = []
        draws = vertices = 0
        batches_per_frame[len(frame['batches'])] += 1
        for batch_index, batch in enumerate(frame['batches']):
            times[f'batch_{batch_index}_duration'].append(batch['duration_ns'] / 1e6)
            starts = collections.defaultdict(list)
            for event in batch['events']:
                name = event['event']
                clean_params = {key: value for key, value in event['params'].items()
                                if key not in ('command_buffer_handle', 'frame')}
                event_counts[name] += 1
                signature.append((batch_index, name, clean_params))
                params[name][json.dumps(clean_params, sort_keys=True)] += 1
                timestamp = int(event['time_ns'])
                if name.startswith('intel_begin_'):
                    starts[name[len('intel_begin_'):]].append(timestamp)
                elif name.startswith('intel_end_'):
                    kind = name[len('intel_end_'):]
                    if starts[kind]:
                        elapsed = timestamp - starts[kind].pop()
                        times[kind].append(elapsed / 1e6)
                if name == 'intel_end_draw':
                    draws += 1
                    vertices += int(clean_params['count'])
        draws_per_frame[draws] += 1
        vertices_per_frame[vertices] += 1
        encoded = json.dumps(signature, sort_keys=True).encode()
        signature_counts[hashlib.sha256(encoded).hexdigest()] += 1
    selected_params = {}
    for name in ('intel_end_draw', 'intel_end_blorp', 'intel_end_render_pass'):
        selected_params[name] = [
            {'params': json.loads(encoded), 'total_count': count,
             'count_per_frame': count / len(frames)}
            for encoded, count in params[name].items()]
    render_times = times['render_pass']
    time_chunks = [render_times[index * len(render_times) // 6:
                                (index + 1) * len(render_times) // 6]
                   for index in range(6)]
    return {
        'file': str(path.resolve()),
        'input_sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
        'exclude_startup_seconds': exclude_seconds,
        'total_complete_frames': len(all_frames),
        'unparsed_tail_characters': tail_chars,
        'steady_frames': len(frames),
        'event_counts': dict(event_counts),
        'event_counts_per_frame': {name: count / len(frames)
                                   for name, count in event_counts.items()},
        'batches_per_frame_histogram': dict(batches_per_frame),
        'draws_per_frame_histogram': dict(draws_per_frame),
        'draw_vertex_count_per_frame_histogram': dict(vertices_per_frame),
        'normalized_signature_sha256_counts': dict(signature_counts),
        'normalization': 'Keep batch index, event name, all params except command_buffer_handle and frame; omit timestamps and durations.',
        'selected_event_parameters': selected_params,
        'interval_durations': {name: distribution(values) for name, values in times.items()},
        'render_pass_six_time_chunk_medians_ms': [statistics.median(chunk)
                                                for chunk in time_chunks if chunk],
        'timing_caution': 'Intervals may overlap. Summing draw/BLORP/render-pass intervals double-counts GPU pipeline work.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--exclude-seconds', type=float, default=1)
    parser.add_argument('captures', type=Path, nargs='+')
    args = parser.parse_args()
    print(json.dumps([summarize(path, args.exclude_seconds) for path in args.captures], indent=2))


if __name__ == '__main__':
    main()
