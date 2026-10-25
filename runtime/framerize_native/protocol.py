"""Small ledger operations. No credentials, browser commands or SDK execution."""
from .owner import require


class Protocol:
    FIELDS = {
        'open': {'account', 'lease'},
        'admit': {'account', 'lease', 'generation', 'budget_ms'},
        'finish': {'account', 'lease', 'generation', 'sequence', 'joined'},
        'close': {'account', 'lease', 'generation', 'joined', 'risk_refused'},
    }

    def __init__(self, owner):
        self.owner, self.grants = owner, {}

    async def call(self, operation, args):
        require(type(operation) is str and operation in self.FIELDS, 'unknown ledger operation')
        require(type(args) is dict and set(args) == self.FIELDS[operation], 'invalid ledger arguments')
        require(type(args['account']) is str and type(args['lease']) is str, 'invalid ledger identity')
        key = args['account'], args['lease']
        if operation == 'open':
            grant = self.owner.open(*key)
            self.grants[key] = grant
            return grant.wire()
        grant = self.grants.get(key)
        require(grant is not None and args['generation'] == grant.generation, 'unknown ledger grant')
        if operation == 'admit':
            return {'sequence': await self.owner.admit_mint(grant, args['budget_ms'])}
        if operation == 'finish':
            ready = self.owner.finish_mint(grant, args['sequence'], args['joined'])
            return {'confirmed': args['joined'], 'ready': ready}
        closed = self.owner.close_grant(grant, args['joined'], args['risk_refused'])
        if closed:
            del self.grants[key]
        return {'closed': closed}
