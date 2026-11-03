"""Exercise actual OBS route decorators without starting the production app."""
import ast
from pathlib import Path

import pytest
from fastapi import Depends, FastAPI
from fastapi.testclient import TestClient
from app.runtime_session import claim_runtime_session, release_runtime_session, runtime_session_dependency


@pytest.mark.parametrize('path,name',[('/api/obs/launch','obs_launch'),('/api/obs/config-check','obs_config_check')])
def test_actual_obs_launch_routes_reject_cosmetic_owner_before_handler(path,name):
    source=ast.parse((Path(__file__).parents[1]/'app/main.py').read_text(encoding='utf-8'))
    original=next(node for node in source.body if isinstance(node,ast.FunctionDef) and node.name==name)
    app=FastAPI();calls=[]
    stub=ast.FunctionDef(name=name,args=ast.arguments(posonlyargs=[],args=[],kwonlyargs=[],kw_defaults=[],defaults=[]),
        body=[ast.Expr(value=ast.Call(func=ast.Attribute(value=ast.Name(id='calls',ctx=ast.Load()),attr='append',ctx=ast.Load()),args=[ast.Constant(True)],keywords=[])),ast.Return(value=ast.Constant('ok'))],
        decorator_list=original.decorator_list)
    namespace={'app':app,'Depends':Depends,'runtime_session_dependency':runtime_session_dependency,'calls':calls}
    exec(compile(ast.fix_missing_locations(ast.Module(body=[stub],type_ignores=[])),'<actual-route-decorator>','exec'),namespace)
    claim=claim_runtime_session('cosmetic-test')
    try:
        response=TestClient(app).post(path)
        assert response.status_code==409
        assert calls==[]
    finally:release_runtime_session(claim)
    assert TestClient(app).post(path).status_code==200
    assert calls==[True]
