"""Pure structural preflight for the generated resident support source."""

import ast
import re

_MAX_SOURCE = 256 * 1024
_BASENAME = re.compile(r'[A-Za-z0-9_.-]{1,255}\Z')


def _run(node):
    return (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
            and node.func.attr == 'run' and isinstance(node.func.value, ast.Name)
            and node.func.value.id == 'resident_control_client')


def _check(tree, expected):
    functions = [node for node in ast.walk(tree)
                 if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
                 and node.name == 'normal_then_control']
    if len(functions) != 1 or type(functions[0]) is not ast.FunctionDef:
        raise ValueError()
    function = functions[0]
    if function not in tree.body or function.decorator_list:
        raise ValueError()
    nodes = list(ast.walk(function))
    stores = [node for node in nodes if isinstance(node, ast.Name)
              and node.id == 'request_path' and not isinstance(node.ctx, ast.Load)]
    if len(stores) != 1:
        raise ValueError()
    assignments = [(index, node) for index, node in enumerate(function.body)
                   if isinstance(node, ast.Assign) and len(node.targets) == 1
                   and node.targets[0] is stores[0]]
    if len(assignments) != 1:
        raise ValueError()
    index, assignment = assignments[0]
    value = assignment.value
    if (not isinstance(value, ast.BinOp) or not isinstance(value.op, ast.Div)
            or not isinstance(value.left, ast.Name) or value.left.id != 'R'
            or not isinstance(value.right, ast.Constant)
            or type(value.right.value) is not str
            or value.right.value != 'private/' + expected):
        raise ValueError()
    calls = [node for node in nodes if _run(node)]
    if len(calls) != 1:
        raise ValueError()
    if not any(position > index and getattr(node, 'value', None) is calls[0]
               for position, node in enumerate(function.body)):
        raise ValueError()


def check_request_path(source: str, expected_basename: str) -> None:
    """Require the literal path before the single direct control call.

    Accept only the generated, undecorated synchronous function shape. Both
    statements must be in its direct body; nested assignments are ambiguous.
    This checks source structure, not runtime globals or execution safety.
    Source is bounded to 256 Ki characters. All ordinary refusals raise only
    ValueError('resident-attempt-path'), without the parser's exception text.
    """
    try:
        if (type(source) is not str or not 0 < len(source) <= _MAX_SOURCE
                or type(expected_basename) is not str
                or not _BASENAME.fullmatch(expected_basename)
                or expected_basename in ('.', '..')):
            raise ValueError()
        _check(ast.parse(source), expected_basename)
        return None
    except Exception:
        pass
    raise ValueError('resident-attempt-path') from None
