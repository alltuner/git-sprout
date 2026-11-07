#!/usr/bin/env python3
"""Unix integration contracts. Only temporary HOME/profiles/repositories are written."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

BINARY = Path(os.environ['SPROUT_BIN']).resolve()
GIT = shutil.which('git')

class Integration(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='sprout-integration-')
        self.addCleanup(self.tmp.cleanup)
        self.home = Path(self.tmp.name)
        self.prefix = self.home / 'shim with quote\''
        self.env = dict(HOME=str(self.home), PATH='/usr/bin:/bin', LC_ALL='C',
                        GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1',
                        GIT_TERMINAL_PROMPT='0')

    def run_cli(self, *args, ok=True, input=None, env=None):
        p = subprocess.run([str(BINARY), *map(str,args)], env=env or self.env,
                           input=input, text=True, capture_output=True)
        if ok:
            self.assertEqual(p.returncode,0,p.stderr + p.stdout)
        else:
            self.assertNotEqual(p.returncode,0,p.stderr + p.stdout)
        return p

    def install(self, *args, apply=True):
        return self.run_cli('install-shim','--prefix',self.prefix,'--real-git',GIT,
                            *(['--apply'] if apply else []),*args)

    def git(self, *args, cwd=None):
        p = subprocess.run([GIT,*map(str,args)],cwd=cwd,env=self.env,text=True,capture_output=True)
        self.assertEqual(p.returncode,0,p.stderr)
        return p.stdout.strip()

    def repo(self):
        repo=self.home/'repo';repo.mkdir();self.git('init',repo)
        self.git('-C',repo,'config','user.name','Test')
        self.git('-C',repo,'config','user.email','test@example.invalid')
        (repo/'tracked').write_text('original\n');(repo/'.gitignore').write_text('cache/\n.claude/\n')
        self.git('-C',repo,'add','.');self.git('-C',repo,'commit','-m','initial')
        return repo

    def hook(self, event, payload, ok=True):
        return self.run_cli('__claude-worktree-'+event,self.prefix/'installation.json',
                            input=json.dumps(payload),ok=ok)

    def test_preview_profiles_preservation_and_uninstall(self):
        profile=self.home/'.codex2';profile.mkdir();(profile/'config.toml').write_text('model="test"\n')
        override=profile/'AGENTS.override.md';override.write_text('Owner override\n')
        empty=self.home/'.codex3';empty.mkdir();(empty/'config.toml').write_text('')
        (empty/'AGENTS.override.md').write_text('')
        claude=self.home/'.claude-no-2';claude.mkdir()
        settings={'env':{'TEST_CREDENTIAL':'opaque-test-fixture'},'mcpServers':{},'hooks':{'SessionStart':[{'hooks':[{'type':'command','command':'true'}]}]}}
        (claude/'settings.json').write_text(json.dumps(settings))
        bash=self.home/'.bashrc';bash.write_text('#!/bin/bash\nreturn\n# user content\n')
        self.install('--agents','--claude-worktrees',apply=False)
        self.assertFalse(self.prefix.exists());self.assertEqual(override.read_text(),'Owner override\n')
        self.install('--agents','--claude-worktrees')
        self.assertIn('git-sprout agent guidance',override.read_text())
        self.assertTrue((empty/'AGENTS.md').exists())
        after=json.loads((claude/'settings.json').read_text())
        self.assertEqual(after['env'],settings['env']);self.assertEqual(after['hooks']['SessionStart'],settings['hooks']['SessionStart'])
        before=override.read_text();self.install('--agents','--claude-worktrees');self.assertEqual(before,override.read_text())
        override.write_text(override.read_text()+'Later user edit\n')
        self.run_cli('uninstall-shim','--prefix',self.prefix,'--apply')
        self.assertEqual(override.read_text(),'Owner override\nLater user edit\n')
        self.assertEqual(json.loads((claude/'settings.json').read_text()),settings)
        self.assertEqual(bash.read_text(),'#!/bin/bash\nreturn\n# user content\n')
        self.assertFalse((empty/'AGENTS.md').exists())
        self.run_cli('uninstall-shim','--prefix',self.prefix,'--apply')
        self.install('--agents')

    def test_foreign_hook_conflict_no_writes(self):
        profile=self.home/'.claude';profile.mkdir()
        contents=json.dumps({'hooks':{'WorktreeCreate':[{'hooks':[{'type':'command','command':'foreign-create'}]}]}})
        (profile/'settings.json').write_text(contents)
        self.run_cli('install-shim','--apply','--agents','--claude-worktrees','--prefix',self.prefix,'--real-git',GIT,ok=False)
        self.assertFalse(self.prefix.exists());self.assertEqual((profile/'settings.json').read_text(),contents)
        self.assertFalse((self.home/'.bashrc').exists())

    def test_shim_exact_passthrough_and_scoped_wrapper(self):
        recorder=self.home/'real-git'
        recorder.write_text('#!/usr/bin/python3\nimport json,os,sys\nopen(os.environ["RECORD"],"w").write(json.dumps(sys.argv[1:]))\nopen(os.environ["RECORD"]+".guard","w").write(os.environ.get("GIT_SPROUT_SHIM_ACTIVE",""))\n')
        recorder.chmod(0o700)
        self.run_cli('install-shim','--apply','--prefix',self.prefix,'--real-git',recorder)
        record=self.home/'record';env=dict(self.env,RECORD=str(record),SPROUT_DISABLE='1')
        # Unknown globals and unsupported add flags must reach real Git unchanged.
        for argv in [['--unknown','worktree','add','a b'],['-C','a b','worktree','add','--unknown','x'],['status','--porcelain'],['worktree','add','--no-cow','x']]:
            p=subprocess.run([str(self.prefix/'git'),*argv],env=env,capture_output=True,text=True)
            self.assertEqual(p.returncode,0,p.stderr);self.assertEqual(json.loads(record.read_text()),argv)
        env['PATH']=str(self.prefix)+':/usr/bin:/bin'
        routed=['-C','a b','worktree','add','-b','branch','new path']
        p=subprocess.run([str(self.prefix/'git'),*routed],env=env,capture_output=True,text=True,timeout=15)
        self.assertEqual(p.returncode,0,p.stderr)
        self.assertEqual(json.loads(record.read_text()),routed)
        self.assertEqual(Path(str(record)+'.guard').read_text(),'1')
        wrapper=self.home/'account-wrapper';wrapper.write_text('#!/bin/sh\nprintf "%s\\n" "$ACCOUNT_PROFILE" "$1"\ncommand -v git\n');wrapper.chmod(0o700)
        env['ACCOUNT_PROFILE']='codex3'
        p=self.run_cli('run','--prefix',self.prefix,'--',wrapper,'argument with space',env=env)
        self.assertEqual(p.stdout.splitlines(),['codex3','argument with space',str(self.prefix/'git')])

    def test_actual_git_shim_worktree_and_guard(self):
        repo=self.repo();self.install()
        env=dict(self.env,PATH=str(self.prefix)+':/usr/bin:/bin',SPROUT_DISABLE='1')
        target=self.home/'new worktree'
        p=subprocess.run([str(self.prefix/'git'),'-C',str(repo),'worktree','add','-b','via-shim',str(target)],env=env,capture_output=True,text=True,timeout=30)
        self.assertEqual(p.returncode,0,p.stderr);self.assertEqual((target/'tracked').read_text(),'original\n')
        self.assertEqual(self.git('-C',target,'branch','--show-current'),'via-shim')

    def test_fallback_when_binary_or_config_unavailable_and_no_retry(self):
        local_binary=self.home/'installed-sprout'
        shutil.copyfile(BINARY,local_binary);local_binary.chmod(0o700)
        recorder=self.home/'record-git';record=self.home/'calls'
        recorder.write_text('#!/usr/bin/python3\nimport json,os,sys\n'
                            +'with open('+repr(str(record))+',"a") as f: f.write(json.dumps(sys.argv[1:])+"\\n")\n'
                            +'sys.exit(int(os.environ.get("GIT_TEST_EXIT","0")))\n')
        recorder.chmod(0o700)
        p=subprocess.run([str(local_binary),'install-shim','--apply','--prefix',str(self.prefix),
                          '--real-git',str(recorder)],env=self.env,capture_output=True,text=True)
        self.assertEqual(p.returncode,0,p.stderr)
        config=self.prefix/'installation.json';original=config.read_text()
        env=dict(self.env,PATH=str(self.prefix)+':/usr/bin:/bin',SPROUT_DISABLE='1')
        cases=['binary-missing','config-deleted','config-malformed','config-inactive','config-bad-schema']
        for case in cases:
            with self.subTest(case=case):
                config.write_text(original)
                if case=='binary-missing': local_binary.rename(self.home/'hidden-sprout')
                elif case=='config-deleted': config.unlink()
                elif case=='config-malformed': config.write_text('{bad')
                elif case=='config-inactive':
                    value=json.loads(original);value['installed']=False;config.write_text(json.dumps(value))
                else: config.write_text(json.dumps({'version':1,'installed':True,'real_git':17}))
                for argv in [['status','--porcelain','a b'],['-C','some dir','worktree','add','-b','test','new path']]:
                    record.unlink(missing_ok=True)
                    p=subprocess.run([str(self.prefix/'git'),*argv],env=env,capture_output=True,text=True,timeout=15)
                    self.assertEqual(p.returncode,0,p.stderr)
                    self.assertEqual([json.loads(line) for line in record.read_text().splitlines()],[argv])
                if case=='binary-missing': (self.home/'hidden-sprout').rename(local_binary)
        config.write_text(original);record.unlink()
        env['GIT_TEST_EXIT']='42'
        argv=['worktree','add','-b','failed','failed path']
        p=subprocess.run([str(self.prefix/'git'),*argv],env=env,capture_output=True,text=True,timeout=15)
        self.assertEqual(p.returncode,42)
        self.assertEqual([json.loads(line) for line in record.read_text().splitlines()],[argv])

    def test_true_empty_unset_path_and_uninstall_activation(self):
        self.install()
        probe=self.home/'path-probe';probe.write_text('#!/usr/bin/python3\nimport os\nprint(os.environ["PATH"])\n');probe.chmod(0o700)
        for unset in [False,True]:
            env=dict(self.env,PATH='')
            if unset: env.pop('PATH')
            p=self.run_cli('run','--prefix',self.prefix,'--',probe,env=env)
            self.assertEqual(p.stdout.strip(),str(self.prefix))
            setup='unset PATH; ' if unset else 'PATH=; export PATH; '
            p=subprocess.run(['/bin/bash','-c',setup+'source "$1"; printf "%s" "$PATH"','bash',str(self.prefix/'activate.sh')],env=env,text=True,capture_output=True)
            self.assertEqual(p.returncode,0,p.stderr);self.assertEqual(p.stdout,str(self.prefix))
        # Explicit nonempty PATH entries (including '.') retain their user-selected policy.
        env=dict(self.env,PATH='.:/usr/bin')
        p=self.run_cli('run','--prefix',self.prefix,'--',probe,env=env)
        self.assertEqual(p.stdout.strip(),str(self.prefix)+':.:/usr/bin')
        self.run_cli('uninstall-shim','--prefix',self.prefix,'--apply')
        p=subprocess.run(['/bin/bash','-c','source "$1"','bash',str(self.prefix/'activate.sh')],env=self.env,capture_output=True,text=True)
        self.assertEqual(p.returncode,0,p.stderr);self.assertEqual(p.stderr,'')

    @unittest.skipUnless(os.name=='posix','requires POSIX byte paths')
    def test_non_utf8_profile_fails_before_installation_writes(self):
        invalid=os.fsencode(self.home)+b'/profile-\xff'
        # Byte argv must actually reach the executable, not a Unicode substitute.
        args=[os.fsencode(BINARY),b'install-shim',b'--apply',b'--agents',
              b'--prefix',os.fsencode(self.prefix),b'--real-git',os.fsencode(GIT),
              b'--codex-home',invalid]
        p=subprocess.run(args,env=self.env,capture_output=True)
        self.assertNotIn(p.returncode,[0,101],p.stderr)
        self.assertIn(b'integration paths must be UTF-8',p.stderr)
        self.assertFalse(self.prefix.exists())
        self.assertFalse((self.home/'.bashrc').exists())
        self.assertFalse(os.path.exists(invalid))

    def test_absent_profiles_explicit_profiles_and_backup_manifest(self):
        bash=self.home/'.bashrc';bash.write_text('owner content\n');bash.chmod(0o640)
        self.install('--agents','--claude-worktrees')
        self.assertFalse((self.home/'.codex').exists());self.assertFalse((self.home/'.claude').exists())
        manifests=list((self.prefix/'backups').glob('*/manifest.json'));self.assertEqual(len(manifests),1)
        rows=json.loads(manifests[0].read_text());row=next(row for row in rows if row['path']==str(bash))
        self.assertEqual(row['mode'],0o640)
        self.assertEqual((manifests[0].parent/row['backup']).read_text(),'owner content\n')
        self.assertEqual(manifests[0].stat().st_mode & 0o777,0o600)
        custom=self.home/'custom-code';active=self.home/'active-claude'
        env=dict(self.env,CLAUDE_CONFIG_DIR=str(active))
        self.run_cli('install-shim','--apply','--agents','--claude-worktrees','--prefix',self.prefix,
                     '--codex-home',custom,env=env)
        self.assertTrue((custom/'AGENTS.md').exists());self.assertTrue((active/'CLAUDE.md').exists())
        self.assertFalse((self.home/'.codex').exists());self.assertFalse((self.home/'.claude').exists())

    def test_native_create_uses_configured_git_outside_path(self):
        repo=self.repo()
        recorder=self.home/'configured-git'
        record=self.home/'configured-git.calls'
        path_record=self.home/'configured-git.paths'
        # Absolute interpreter/real Git keep this recorder functional with an empty PATH.
        recorder.write_text('#!/usr/bin/python3\nimport json,os,sys\n'
                            +'with open('+repr(str(record))+',"a") as f: f.write(json.dumps(sys.argv[1:])+"\\n")\n'
                            +'if "worktree" in sys.argv and "add" in sys.argv:\n with open('+repr(str(path_record))+',"a") as f: f.write(os.environ.get("PATH","<unset>")+"\\n")\n'
                            +'if os.environ.get("FAIL_MARKER") and "--absolute-git-dir" in sys.argv: print("/dev/null"); sys.exit(0)\n'
                            +'os.execv('+repr(GIT)+',['+repr(GIT)+']+sys.argv[1:])\n')
        recorder.chmod(0o700)
        self.run_cli('install-shim','--apply','--agents','--claude-worktrees',
                     '--prefix',self.prefix,'--real-git',recorder)
        for unset in [False,True]:
            env=dict(self.env,PATH='',SPROUT_DISABLE='1')
            if unset: env.pop('PATH')
            slug='configured-'+str(unset)
            result=self.run_cli('__claude-worktree-create',self.prefix/'installation.json',
                                input=json.dumps({'cwd':str(repo),'name':slug}),env=env)
            target=Path(result.stdout.strip())
            self.assertEqual((target/'tracked').read_text(),'original\n')
        calls=[json.loads(line) for line in record.read_text().splitlines()]
        self.assertTrue(any('worktree' in args and 'add' in args for args in calls),calls)
        self.assertEqual(sum('worktree' in args and 'add' in args for args in calls),2)
        self.assertEqual(path_record.read_text().splitlines(),[str(self.prefix)]*2)
        env=dict(self.env,PATH='',SPROUT_DISABLE='1',FAIL_MARKER='1')
        orphan=repo/'.claude/worktrees/marker-failure'
        result=self.run_cli('__claude-worktree-create',self.prefix/'installation.json',
                            input=json.dumps({'cwd':str(repo),'name':'marker-failure'}),env=env,ok=False)
        self.assertIn(str(orphan),result.stderr)
        self.assertTrue((orphan/'tracked').exists())
        self.assertEqual(self.git('-C',orphan,'branch','--show-current'),'claude/marker-failure')


    def test_native_claude_lifecycle_keeps_dirty_ignored_and_branches(self):
        repo=self.repo();self.install('--agents','--claude-worktrees')
        for slug,dirty in [('clean',None),('dirty','tracked'),('untracked','new'),('ignored','cache/data')]:
            result=self.hook('create',{'cwd':str(repo),'name':slug})
            target=Path(result.stdout.strip());self.assertEqual(target,repo/'.claude/worktrees'/slug)
            if dirty:
                path=target/dirty;path.parent.mkdir(parents=True,exist_ok=True);path.write_text('keep me')
            self.hook('remove',{'worktree_path':str(target)},ok=not bool(dirty))
            self.assertEqual(target.exists(),bool(dirty))
            if dirty:
                self.assertEqual((target/dirty).read_text(),'keep me')
            else:
                self.hook('remove',{'worktree_path':str(target)}) # already absent is success
            self.assertEqual(self.git('-C',repo,'branch','--format=%(refname:short)','--list','claude/'+slug).strip(),'claude/'+slug)
        self.hook('create',{'cwd':str(repo),'name':'../escape'},ok=False)
        self.assertFalse((repo/'.claude/escape').exists())
        outside=self.home/'outside';outside.mkdir()
        self.hook('remove',{'worktree_path':str(repo)},ok=False) # refuse unowned worktree
        self.assertTrue(repo.exists())
        (repo/'.claude/worktrees/link').symlink_to(outside,target_is_directory=True)
        self.hook('create',{'cwd':str(repo),'name':'link'},ok=False)
        self.assertEqual(list(outside.iterdir()),[])

if __name__=='__main__':
    unittest.main()
