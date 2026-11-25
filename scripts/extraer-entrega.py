import sys,base64,re
log=open(sys.argv[1],encoding='utf-8',errors='replace').read().splitlines()
cur=None;buf=[]
ts=re.compile(r'^\d{4}-\d\d-\d\dT[\d:.]+Z ')
for l in log:
    l=ts.sub('',l.lstrip('\ufeff'))
    if l.startswith('@@INICIO '): cur=l[9:].strip(); buf=[]; continue
    if l.startswith('@@FIN ') and cur:
        open(cur,'wb').write(base64.b64decode(''.join(buf))); print('ok',cur); cur=None; continue
    if cur: buf.append(re.sub(r'[^A-Za-z0-9+/=]','',l))
