"""Assemble acknowledged browser chunks through an account-owned poll call."""
import asyncio
import time

BODY_LIMIT = 8 * 1024**2
QUEUE_CAPACITY = 131072


class ResponseDeadline(TimeoutError):
    pass


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


async def receive(poll, deadline):
    """Poll retains its actual RPC owner even when this waiter times out."""
    acknowledged, offset, header, final = -1, 0, None, None
    parts, delay = [], 0.005
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise ResponseDeadline('browser response deadline exceeded')
        try:
            value = await asyncio.wait_for(poll(acknowledged), min(8, remaining))
        except asyncio.TimeoutError:
            continue
        require(type(value) is dict, 'invalid browser response envelope')
        kind = value.get('kind')
        if final is not None:
            require(kind == 'missing' or (kind == 'closed' and
                    type(value.get('sequence')) is int and
                    value['sequence'] == acknowledged), 'final acknowledgement failed')
            return final
        require(kind in ('pending', 'chunk'), 'browser response disappeared before completion')
        if kind == 'pending':
            await asyncio.sleep(min(delay, max(0, deadline - time.monotonic())))
            delay = min(delay * 2, 0.25)
            continue
        require(type(value.get('sequence')) is int and
                value['sequence'] == acknowledged + 1, 'browser response sequence changed')
        require(type(value.get('offset')) is int and value['offset'] == offset,
                'browser response offset changed')
        require(type(value.get('next')) is int, 'invalid browser response next offset')
        require(type(value.get('done')) is bool and type(value.get('body')) is str
                and type(value.get('status')) is int and
                (value.get('usage') is None or type(value['usage']) is str),
                'invalid browser response fields')
        if value['status'] == -1:
            require(value['done'] and value['next'] == offset,
                    'invalid terminal browser response failure')
            parts.clear()
            final = {key: value[key] for key in
                     ('status', 'usage', 'body', 'failure_phase', 'head_status')
                     if key in value}
        else:
            current_header = (value['status'], value.get('usage'))
            require(header is None or header == current_header,
                    'browser response status or usage changed')
            header = current_header
            units = len(value['body'].encode('utf-16-le')) // 2
            require(units <= QUEUE_CAPACITY and (units > 0 or value['done']) and
                    value['next'] == offset + units, 'invalid browser response chunk')
            require(value['next'] <= BODY_LIMIT, 'browser response exceeded limit')
            require(type(value.get('peak_units')) is int and
                    0 <= value['peak_units'] <= QUEUE_CAPACITY,
                    'browser response queue exceeded limit')
            parts.append(value['body'])
            offset = value['next']
            if value['done']:
                final = dict(status=header[0], usage=header[1], body=''.join(parts))
                parts.clear()
        acknowledged, delay = value['sequence'], 0.005
