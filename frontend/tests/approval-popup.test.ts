import test from 'node:test';
import assert from 'node:assert/strict';
import { validApprovalMessage } from '../src/approval-popup';
const popup = {} as Window, origin = "https://app.example", state = "a".repeat(43);
const valid = { origin, source: popup, data: { type: "silicon:feature-approval", state, code: "obc_oneuse" } };
test('feature callback requires exact window, origin, request state and one-use code', () => {
 assert.equal(validApprovalMessage(valid,origin,popup,state),true);
 for(const event of [{...valid,source:{} as Window},{...valid,origin:'https://evil.example'},{...valid,data:{...valid.data,state:'b'.repeat(43)}},{...valid,data:{...valid.data,code:'oba_access'}}]) assert.equal(validApprovalMessage(event,origin,popup,state),false);
});
