"""Receive bounded chunks without acknowledging an unreceived RPC reply."""
import asyncio
from pathlib import Path

POLL = (Path(__file__).resolve().parent / 'queue_ack_poll.js').read_text()
CAPACITY = 131072
BODY_LIMIT = 8 * 1024**2


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


async def drain(page, job, *, timeout=30, poll_timeout=5):
    """The caller retains the job owner and must join cancellation on failure."""
    end = asyncio.get_running_loop().time() + timeout
    acknowledged, offset, header, final = -1, 0, None, None
    parts = []
    metrics = dict(calls=0, retries=0, frames=0, empty_polls=0,
                   peak_queue_units=0, received_units=0)
    while metrics['calls'] < 5000:
        remaining = end - asyncio.get_running_loop().time()
        require(remaining > 0, 'response acknowledgement deadline exceeded')
        metrics['calls'] += 1
        try:
            value = await asyncio.wait_for(
                page.evaluate(POLL, [job, acknowledged]), min(poll_timeout, remaining))
        except asyncio.TimeoutError:
            metrics['retries'] += 1
            continue
        require(type(value) is dict, 'invalid response delivery envelope')
        kind = value.get('kind')
        if final is not None:
            require(kind == 'missing' or (kind == 'closed' and
                    type(value.get('sequence')) is int and
                    value['sequence'] == acknowledged), 'final acknowledgement failed')
            return final, metrics
        require(kind in ('pending', 'chunk'), 'response job disappeared before completion')
        if kind == 'pending':
            metrics['empty_polls'] += 1
            await asyncio.sleep(min(0.005, remaining))
            continue
        require(type(value.get('sequence')) is int and
                value['sequence'] == acknowledged + 1, 'response sequence changed')
        require(type(value.get('offset')) is int and value['offset'] == offset,
                'response offset changed')
        require(type(value.get('done')) is bool and type(value.get('body')) is str
                and type(value.get('status')) is int and
                (value.get('usage') is None or type(value['usage']) is str),
                'invalid response chunk fields')
        require(type(value.get('next')) is int, 'invalid next response offset')
        if value['status'] == -1:
            require(value['done'] and value['next'] == offset,
                    'invalid terminal response failure')
            parts.clear()
            final = dict(value)
        else:
            current_header = (value['status'], value.get('usage'))
            require(header is None or header == current_header,
                    'response status or usage changed')
            header = current_header
            units = len(value['body'].encode('utf-16-le')) // 2
            require(units <= CAPACITY and value['next'] == offset + units,
                    'response chunk exceeded bounds or lost content')
            require(value['next'] <= BODY_LIMIT, 'decoded response exceeded limit')
            require(type(value.get('peak_units')) is int and
                    0 <= value['peak_units'] <= CAPACITY, 'response queue exceeded limit')
            metrics['peak_queue_units'] = max(metrics['peak_queue_units'], value['peak_units'])
            parts.append(value['body'])
            offset = value['next']
            metrics['received_units'] = offset
            if value['done']:
                final = dict(value, body=''.join(parts))
                parts.clear()
        metrics['frames'] += 1
        acknowledged = value['sequence']
    raise RuntimeError('response acknowledgement poll bound exceeded')
