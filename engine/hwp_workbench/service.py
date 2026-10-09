from pathlib import Path
from .document import Document,sha
from .errors import require
from .workspace import Workspace

CAPABILITIES={
    'version':'0.1.0-alpha.1',
    'formats':{'hwp':'HWP 5.x: existing text, equal UTF-16 length only','hwpx':'Existing plain text nodes; variable length; same-format save'},
    'operations':['inspect','extract','validate','plan','apply'],
    'notImplemented':['native full-page rendering','formatting and structural edits','new document authoring','HWP/HWPX conversion','DOCX/PDF export','comments/track changes','full Hancom fidelity'],
    'safety':{'overwrite':False,'externalNetwork':False,'execDocumentCode':False,'workspaceRestricted':True,'requireRiskAcknowledgement':True},
}

def invoke(root,method,args):
    require(isinstance(args,dict),'BAD_ARGUMENTS','Arguments must be an object.')
    if method=='capabilities': return CAPABILITIES
    ws=Workspace(root)
    path=args.get('path')
    data=ws.read(path)
    doc=Document(data,Path(path).name)
    if method=='inspect': return doc.inspect()
    if method=='extract':
        return {'format':doc.format,'sha256':doc.hash,'text':'\n'.join(n.text for n in doc.nodes),'warnings':doc.warnings}
    if method=='validate':
        return {'valid':True,'scope':'bounded container/XML/record parsing only; not Hancom rendering certification',
                'sha256':doc.hash,'format':doc.format,'textNodes':len(doc.nodes),'warnings':doc.warnings}
    require(method in {'plan','apply'},'UNKNOWN_METHOD','Unknown operation: '+method)
    doc.patch(args.get('patch'))
    result=doc.diff()
    if method=='plan': return result
    out=args.get('output')
    require(isinstance(out,str) and Path(out).suffix.lower()=='.'+doc.format,'OUTPUT_FORMAT','Output must use the same native extension as the source.')
    require(args.get('acknowledgeRisks') is True,'RISK_ACK_REQUIRED','Pass acknowledgeRisks: true to save edited output.')
    content=doc.serialize(True)
    from .independent import check_output
    independent_check = check_output(content, doc.edits)
    # Optimistic concurrency check immediately before publication; sources never get overwritten.
    require(sha(ws.read(path))==doc.hash,'STALE_DOCUMENT','Source changed during processing; output was not published.')
    ws.write_new(out,content)
    return result|{'output':out,'outputSha256':sha(content),'reopenedAndVerified':True,'sourceUntouched':True,'independentCheck':independent_check}
