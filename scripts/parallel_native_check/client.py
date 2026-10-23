import http.client
import json
import uuid
from .guard import ADDRESS
from .streaming import Stream
from .support import require


def pool():
    connection = http.client.HTTPConnection(*ADDRESS, timeout=3)
    try:
        connection.request('GET', '/api/pool', headers={'Connection': 'close'})
        response = connection.getresponse()
        body = response.read(131073)
        require(response.status == 200 and len(body) <= 131072, 'pool_response')
        return json.loads(body)
    finally:
        connection.close()


def head():
    connection = http.client.HTTPConnection(*ADDRESS, timeout=3)
    try:
        connection.request('HEAD', '/anthropic/api/hello', headers={'Connection': 'close'})
        response = connection.getresponse()
        body = response.read(1)
        require(response.status == 200 and not body and
                response.getheader('Content-Length') == '2', 'hello_head')
        return {'status': 200, 'body_bytes': 0, 'content_length': 2}
    finally:
        connection.close()


def send(binding, cancel, report):
    require(report['post_calls'] < 2, 'post_limit')
    prompt = 'Count from one to five hundred in words.' if cancel else 'Reply with exactly OK.'
    payload = {'model': 'gpt-4.1', 'input': prompt, 'stream': True,
               'max_output_tokens': 512 if cancel else 16}
    connection = http.client.HTTPConnection(*ADDRESS, timeout=65)
    report['post_calls'] += 1
    try:
        connection.request('POST', '/openai/v1/responses', body=json.dumps(payload), headers={
            'Content-Type': 'application/json', 'Connection': 'close',
            'x-framerize-account': binding.account, 'x-framerize-session': uuid.uuid4().hex,
            'x-framerize-context': json.dumps(binding.input['context'])})
        return Stream(connection)
    except BaseException:
        connection.close()
        raise
