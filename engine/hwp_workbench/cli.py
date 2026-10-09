"""Executable without third-party Python dependencies."""
import argparse,json,sys
from pathlib import Path
from .errors import WorkbenchError,require
from .service import invoke,CAPABILITIES
from .mcp import serve
from .workspace import Workspace

def main(argv=None):
    parser=argparse.ArgumentParser(prog='hwp-workbench',description='Local HWP/HWPX preservation editor — development alpha.')
    parser.add_argument('--version',action='version',version=CAPABILITIES['version'])
    sub=parser.add_subparsers(dest='command',required=True)
    sub.add_parser('capabilities')
    for cmd in ['inspect','extract','validate','plan','apply']:
        p=sub.add_parser(cmd);p.add_argument('path');p.add_argument('--root',default='.',help='Allowed workspace (default: current directory).')
        if cmd in {'plan','apply'}:p.add_argument('--patch',required=True,help='JSON patch file inside workspace.')
        if cmd=='apply':p.add_argument('-o','--output',required=True);p.add_argument('--acknowledge-risks',action='store_true')
        if cmd=='extract':p.add_argument('--format',choices=['text','json'],default='json')
    m=sub.add_parser('mcp');m.add_argument('--root',required=True)
    rpc=sub.add_parser('rpc');rpc.add_argument('--root',required=True,help='One JSON request on stdin; desktop adapter only.')
    args=parser.parse_args(argv)
    try:
        if args.command=='mcp': Workspace(args.root);serve(args.root);return 0
        if args.command=='capabilities': result=CAPABILITIES
        elif args.command=='rpc':
            raw=sys.stdin.read(4*1024*1024+1);require(len(raw)<=4*1024*1024,'LIMIT','Request exceeds limit.')
            request=json.loads(raw);result=invoke(args.root,request['method'],request.get('args',{}))
        else:
            payload={'path':args.path}
            if args.command in {'plan','apply'}:payload['patch']=json.loads(Workspace(args.root).read(args.patch))
            if args.command=='apply':payload.update(output=args.output,acknowledgeRisks=args.acknowledge_risks)
            result=invoke(args.root,args.command,payload)
        if args.command=='extract' and args.format=='text':print(result['text'])
        else:print(json.dumps(result,ensure_ascii=False,indent=2))
        return 0
    except (WorkbenchError,OSError,ValueError,KeyError,TypeError) as e:
        error=e.as_dict() if isinstance(e,WorkbenchError) else {'code':'REQUEST_FAILED','message':str(e)}
        print(json.dumps({'error':error},ensure_ascii=False),file=sys.stderr)
        return 2

if __name__=='__main__':sys.exit(main())
