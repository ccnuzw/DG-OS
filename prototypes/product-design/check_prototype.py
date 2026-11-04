"""Browser checks for the design prototype, not production acceptance tests.

Run with a local server on port 4179 and Python Playwright installed.
Screenshots go to --output; no credentials or external services are used.
"""
import argparse
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser()
parser.add_argument('--url', default='http://127.0.0.1:4179/')
parser.add_argument('--output', required=True)
args = parser.parse_args()
output = Path(args.output)
output.mkdir(parents=True, exist_ok=True)

with sync_playwright() as p:
    browser = p.chromium.launch()
    context = browser.new_context(viewport={'width': 1440, 'height': 1000})
    page = context.new_page()
    errors = []
    external = []
    context.on('page', lambda child: child.on('pageerror', lambda err: errors.append(str(err))))
    page.on('pageerror', lambda err: errors.append(str(err)))
    context.on('request', lambda request: external.append(request.url) if not request.url.startswith(args.url) else None)
    passed = []

    def action(name, target=page):
        target.locator(f'[data-action="{name}"]').first.click()

    def ws_action(name, target=page):
        target.locator(f'[data-ws-action="{name}"]').first.click()

    def scene(name):
        if not page.locator('.review-panel').count():
            action('review')
        action('scene:' + name)

    def settings(section='providers'):
        action('settings:providers')
        if section != 'providers':
            action('section:' + section)

    def no_overflow(target=page):
        assert target.evaluate('document.documentElement.scrollWidth <= innerWidth + 1'), 'Page has horizontal overflow'

    def no_page_scroll(target=page):
        assert target.evaluate('document.documentElement.scrollHeight <= innerHeight + 1'), 'Page has vertical overflow'
        assert target.locator('.composer').evaluate("e=>e.getBoundingClientRect().bottom <= innerHeight + 1"), 'Composer is below the viewport'

    page.goto(args.url)
    page.wait_for_load_state('networkidle')
    no_page_scroll()

    # Hover background belongs to the tab wrapper, not the inner button.
    first_tab = page.locator('.workspace-tab').first
    first_tab.hover()
    page.wait_for_timeout(180)
    assert first_tab.evaluate("e => getComputedStyle(e).backgroundColor !== 'rgba(0, 0, 0, 0)'")
    assert first_tab.locator('.workspace-tab-main').evaluate("e => getComputedStyle(e).backgroundColor === 'rgba(0, 0, 0, 0)'")

    # Command palette is keyboard reachable and searchable.
    page.locator('.command-trigger').click()
    expect(page.locator('.command-panel')).to_be_visible()
    page.locator('#command-input').fill('画布')
    expect(page.locator('.command-item .command-label')).to_have_text(['无限画布'])
    page.keyboard.press('Escape')
    expect(page.locator('.command-panel')).to_have_count(0)
    page.locator('[data-ws-action="tab-menu"]').click()
    expect(page.locator('.ws-tab-menu')).to_be_visible()
    expect(page.locator('.ws-tab-menu-row')).to_have_count(1)
    page.keyboard.press('Escape')
    expect(page.locator('.ws-tab-menu')).to_have_count(0)
    expect(page.get_by_role('heading', name='AI 会话', exact=True)).to_be_visible()
    expect(page.locator('#reply-execution')).to_be_visible()
    expect(page.get_by_role('button', name='选择 Provider 和模型')).to_be_visible()
    expect(page.get_by_role('button', name='选择 Skills')).to_be_visible()
    expect(page.get_by_role('button', name='会话设置')).to_be_visible()
    expect(page.locator('.shell > .sidebar')).to_be_visible()
    expect(page.locator('.session-rail')).to_have_count(0)
    composer_width = page.locator('.agent-composer-shell').bounding_box()['width']
    assert 700 <= composer_width <= 740, composer_width
    page.screenshot(path=str(output / '00-workbench.png'))
    no_overflow()
    passed.append('会话页窄图标栏、执行记录与固定输入区')

    page.get_by_role('button', name='选择 Provider 和模型').click()
    expect(page.get_by_role('dialog', name='选择模型')).to_be_visible()
    page.get_by_role('button', name='Anthropic').click()
    expect(page.get_by_role('button', name='Claude Sonnet 4')).to_be_visible()
    page.get_by_role('button', name='Claude Sonnet 4').click()
    expect(page.get_by_role('button', name='选择 Provider 和模型')).to_contain_text('Claude Sonnet 4')
    page.get_by_role('button', name='选择 Skills').click()
    page.get_by_role('button', name='需求梳理').click()
    expect(page.locator('.composer-context')).to_contain_text('需求梳理')
    page.get_by_role('button', name='会话设置').click()
    page.get_by_role('button', name='深入', exact=True).click()
    expect(page.get_by_role('button', name='思考强度：深入')).to_be_visible()
    page.keyboard.press('Escape')
    textarea = page.get_by_label('消息', exact=True)
    textarea.focus()
    assert page.locator('.agent-composer-shell').evaluate("node => node.matches(':has(#chat-input:focus)')")
    assert page.locator('.agent-composer-shell').evaluate("node => getComputedStyle(node).transform !== 'none'")
    assert textarea.evaluate("node => getComputedStyle(node).borderTopWidth") == '0px'
    textarea.blur()
    action('composer-action:record')
    expect(page.get_by_role('button', name='停止录音演示')).to_have_attribute('aria-pressed', 'true')
    action('composer-action:record')
    action('reply-files-list')
    expect(page.get_by_role('dialog', name='修改文件 · 7 项')).to_be_visible()
    page.locator('#dialog [data-action="reply-file:GenTableForm.vue"]').click(force=True)
    expect(page.get_by_role('dialog', name='GenTableForm.vue')).to_be_visible()
    action('cancel-dialog')
    passed.append('Provider/模型、Skills、会话设置、录音、文件差异与无内边线')

    page.get_by_label('消息', exact=True).fill('标签一草稿')
    action('chat-tab:2')
    expect(page.get_by_label('消息', exact=True)).to_have_value('')
    page.get_by_label('消息', exact=True).fill('标签二草稿')
    action('chat-tab:1')
    expect(page.get_by_label('消息', exact=True)).to_have_value('标签一草稿')
    page.get_by_role('tab', name='产品体验整理', exact=True).press('ArrowRight')
    expect(page.get_by_label('消息', exact=True)).to_have_value('标签二草稿')
    action('new-chat')
    expect(page.get_by_role('tab')).to_have_count(3)
    page.get_by_label('消息', exact=True).fill('标签三草稿')
    page.locator('.conversation-tab').nth(2).hover()
    action('close-chat:3')
    expect(page.get_by_label('消息', exact=True)).to_have_value('标签二草稿')
    action('page:desktop')
    expect(page.get_by_role('heading', name='桌面总览', exact=True)).to_be_visible()
    action('page:workbench')
    expect(page.get_by_label('消息', exact=True)).to_have_value('标签二草稿')
    passed.append('会话草稿隔离、键盘切换、关闭、新建与工作区往返')
    action('settings:providers')
    expect(page.get_by_role('dialog')).to_be_visible()
    action('close-settings')

    # An unsaved form cannot disappear through Escape or classification changes.
    page.get_by_label('消息', exact=True).fill('应保留的工作草稿')
    settings()
    page.get_by_label('显示名称', exact=True).fill('待保存的服务')
    page.keyboard.press('Escape')
    expect(page.get_by_role('dialog')).to_be_visible()
    action('section:mcp')
    expect(page.get_by_label('显示名称', exact=True)).to_have_value('待保存的服务')
    action('save-provider')
    page.screenshot(path=str(output / '02-settings.png'))
    action('close-settings')
    expect(page.get_by_label('消息', exact=True)).to_have_value('应保留的工作草稿')
    passed.append('未保存修改拦截、保存与工作草稿保留')

    # A running request survives cancelled navigation and stops on confirmed leave.
    page.get_by_label('消息', exact=True).press('Enter')
    action('page:skills')
    expect(page.get_by_role('heading', name='离开将停止当前任务')).to_be_visible()
    action('cancel-dialog')
    expect(page.get_by_role('button', name='模拟完成', exact=True)).to_be_visible()
    action('page:skills')
    action('leave:page:skills')
    expect(page.get_by_role('heading', name='技能中心', exact=True)).to_be_visible()
    passed.append('运行中离开确认与取消')

    # Search, add and consume one skill; no pseudo installation is asserted.
    page.get_by_label('搜索技能').fill('不存在的技能')
    expect(page.get_by_role('heading', name='没有匹配的技能')).to_be_visible()
    action('clear-search')
    action('skill:1')
    action('add-skill')
    page.screenshot(path=str(output / '03-skills.png'))
    action('use-skill')
    expect(page.locator('#task-skill')).to_have_value('清晰写作')
    page.locator('#task-tool').check()
    action('run-task')
    expect(page.get_by_role('heading', name='需要你的确认')).to_be_visible()
    page.screenshot(path=str(output / '04-tool-approval.png'))
    action('deny-tool')
    expect(page.get_by_role('heading', name='已拒绝工具调用')).to_be_visible()
    action('run-task')
    action('allow-tool')
    action('finish-task')
    expect(page.get_by_role('heading', name='任务结果', exact=True)).to_be_visible()
    passed.append('技能筛选添加使用、工具拒绝/批准与结果')

    # Permissions and availability affect the product entry.
    settings('modules')
    expect(page.locator('.module-card')).to_have_count(3)
    action('module-install')
    expect(page.locator('[data-action="module-crash"]')).to_be_visible()
    action('module-crash')
    expect(page.locator('[data-action="module-recover"]')).to_be_visible()
    action('module-recover')
    action('module-remove')
    expect(page.locator('[data-action="module-install"]')).to_be_visible()
    action('toggle-permission')
    action('open-app')
    expect(page.get_by_text('此应用尚未获得模型调用权限。', exact=True)).to_be_visible()
    action('settings:modules')
    action('toggle-permission')
    action('toggle-module')
    action('close-settings')
    expect(page.locator('.sidebar [data-ws-action="open-app:model"]')).to_have_count(0)
    settings('modules')
    action('toggle-module')
    action('open-app')
    page.locator('#app-input').fill('测试输入')
    action('app-run')
    expect(page.locator('#app-result')).to_contain_text('模拟调用完成')
    passed.append('应用权限和启停入口')

    # Workspace shell: outer tabs, launcher, focus mode and app tab close.
    action('page:workbench')
    expect(page.locator('.workspace-tabs')).to_be_visible()
    ws_action('launcher')
    expect(page.locator('.launchpad-panel')).to_be_visible()
    expect(page.locator('.launch-item')).to_have_count(8)
    expect(page.locator('.launch-item').first).to_be_visible()
    expect(page.locator('.launchpad-close')).to_have_count(0)
    page.locator('.launchpad-layer').click(position={'x': 5, 'y': 5})
    expect(page.locator('.launchpad-layer')).to_have_count(0)
    ws_action('launcher')
    page.locator('[data-ws-drag-app="tools"]').drag_to(page.locator('.sidebar'))
    expect(page.locator('.sidebar [data-ws-action="open-app:tools"]')).to_have_count(1)
    ws_action('open-app:model')
    expect(page.locator('.workspace-tab [data-ws-action="close-app"]')).to_have_count(1)
    ws_action('toggle-focus')
    assert page.locator('.shell').evaluate("e=>e.classList.contains('ws-focus')")
    assert page.locator('.sidebar').evaluate("e=>getComputedStyle(e).display") == 'none'
    ws_action('toggle-focus')
    page.locator('[data-ws-action="close-app"]').evaluate('e=>e.click()')
    expect(page.locator('[data-ws-action="close-app"]')).to_have_count(0)
    passed.append('工作区标签、启动台、专注模式与应用标签')

    # Skills center and MCP management resource pages.
    action('page:skills')
    expect(page.locator('.resource-head h1')).to_contain_text('技能中心')
    expect(page.locator('.skill-card')).to_have_count(4)
    expect(page.get_by_role('tab', name='技能中心')).to_have_count(1)
    expect(page.locator('[data-ws-action="mcp-page"]')).to_have_count(1)
    ws_action('mcp-page')
    expect(page.locator('.mcp-layout')).to_be_visible()
    expect(page.get_by_role('tab', name='MCP 管理')).to_have_count(1)
    expect(page.locator('.mcp-server-card')).to_have_count(3)
    page.locator('[data-ws-mcp-action^="tool-open:"]').first.click()
    expect(page.locator('.mcp-tool-call')).to_be_visible()
    page.locator('[data-ws-mcp-action="tool-review"]').click()
    expect(page.locator('.tool-review')).to_be_visible()
    page.locator('[data-ws-mcp-action="tool-run"]').click()
    expect(page.locator('.tool-result-box')).to_be_visible()
    page.locator('[data-ws-action="close-mcp"]').evaluate('e=>e.click()')
    expect(page.get_by_role('tab', name='MCP 管理')).to_have_count(0)
    assert page.evaluate('document.documentElement.scrollHeight <= innerHeight + 1')
    passed.append('技能中心与 MCP 管理资源页及标签')

    # Canvas creation platform: project home, create, full-screen editor and back.
    ws_action('launcher')
    page.locator('.launch-item[data-ws-drag-app="canvas"]').click()
    expect(page.locator('.canvas-platform')).to_be_visible()
    expect(page.get_by_role('tab', name='无限画布')).to_have_count(1)
    assert page.locator('.sidebar').evaluate("e=>getComputedStyle(e).display") == 'none'
    page.locator('[data-ws-canvas-action="create"]').click()
    page.locator('#canvas-project-name').fill('检查项目')
    page.locator('[data-ws-canvas-action="create-submit"]').click()
    expect(page.locator('.canvas-editor')).to_be_visible()
    expect(page.locator('.canvas-back')).to_be_visible()
    expect(page.locator('.ws-dirty-dot')).to_have_count(1)
    page.locator('[data-ws-action="close-canvas"]').evaluate('e=>e.click()')
    expect(page.locator('.ws-close-dialog')).to_be_visible()
    page.locator('[data-ws-action="cancel-close-canvas"]').click()
    expect(page.locator('.canvas-editor')).to_be_visible()
    page.locator('.canvas-back').click()
    expect(page.locator('.canvas-platform')).to_be_visible()
    expect(page.locator('.canvas-project-card')).to_have_count(3)
    page.locator('[data-ws-action="close-canvas"]').evaluate('e=>e.click()')
    expect(page.get_by_role('tab', name='无限画布')).to_have_count(0)
    passed.append('画布创作平台、项目创建与全屏编辑器')

    # First-run flow cannot send before configuration.
    scene('first')
    expect(page.get_by_role('heading', name='桌面总览')).to_be_visible()
    action('page:workbench')
    expect(page.get_by_role('heading', name='先连接一个模型服务')).to_be_visible()
    expect(page.get_by_label('消息', exact=True)).to_be_disabled()
    settings()
    page.get_by_label('显示名称', exact=True).fill('')
    action('save-provider')
    expect(page.get_by_role('alert')).to_contain_text('填写名称')
    page.get_by_label('显示名称', exact=True).fill('演示模型服务')
    action('fail-provider')
    expect(page.get_by_role('alert')).to_contain_text('连接失败')
    action('test-provider')
    expect(page.locator('#provider-test')).to_contain_text('连接成功（模拟）')
    action('save-provider')
    action('close-settings')
    expect(page.get_by_role('heading', name='今天，想从哪里开始？')).to_be_visible()
    expect(page.get_by_label('消息', exact=True)).to_be_enabled()
    passed.append('首次配置与表单校验、连接失败/成功状态')

    # Each settings category is reachable, including dark and scaled controls.
    settings('mcp')
    expect(page.get_by_text('完整管理已移至左侧', exact=False)).to_be_visible()
    action('close-settings')
    settings('appearance')
    page.locator('[data-appearance-action="mode:dark"]').click()
    page.locator('[data-appearance-action="apply"]').click()
    expect(page.locator('html')).to_have_attribute('data-theme', 'dark')
    page.screenshot(path=str(output / '05-appearance-dark.png'))
    page.locator('[data-appearance-action="mode:light"]').click()
    page.locator('[data-appearance-action="apply"]').click()
    action('section:about')
    expect(page.get_by_text('工程版本基线', exact=True)).to_be_visible()
    action('close-settings')
    passed.append('全部设置入口与主题')

    # Settings always stays in the main window. UI sizing must not resize content.
    page.get_by_label('消息', exact=True).fill('独立窗口保留的草稿')
    settings('appearance')
    page.locator('[data-appearance-field="fullPreview"]').check()
    expect(page.get_by_text('主窗口内悬浮', exact=True)).to_be_visible()
    assert len(context.pages) == 1
    ui_before = page.locator('.workspace-tabs').bounding_box()['height']
    text_before = page.locator('#chat-input').evaluate('e=>getComputedStyle(e).fontSize')
    page.locator('#density').select_option('1.25')
    assert page.locator('.workspace-tabs').bounding_box()['height'] > ui_before
    assert page.locator('#chat-input').evaluate('e=>getComputedStyle(e).fontSize') == text_before
    ui_after = page.locator('.workspace-tabs').bounding_box()['height']
    page.locator('#content-size').select_option('18')
    assert page.locator('#chat-input').evaluate('e=>getComputedStyle(e).fontSize') == '18px'
    assert page.locator('.workspace-tabs').bounding_box()['height'] == ui_after
    page.locator('#density').select_option('1')
    page.locator('#content-size').select_option('13')
    action('close-settings')
    expect(page.get_by_label('消息', exact=True)).to_have_value('独立窗口保留的草稿')
    settings('lan')
    action('toggle-lan')
    action('remote-preview')
    page.locator('#pair-code').fill('invalid')
    page.get_by_role('button', name='配对并进入').click()
    expect(page.get_by_role('alert')).to_contain_text('配对码无效')
    page.locator('#pair-code').fill('demo')
    page.get_by_role('button', name='配对并进入').click()
    page.locator('#remote-input').fill('远程演示消息')
    page.get_by_role('button', name='发送', exact=True).click()
    expect(page.get_by_text('远程演示消息', exact=True)).to_be_visible()
    expect(page.locator('.sidebar')).to_have_count(0)
    scene('desktop')
    settings('lan')
    action('revoke')
    action('close-settings')
    scene('remote')
    expect(page.get_by_role('heading', name='连接你的 DGOS')).to_be_visible()
    passed.append('单窗口设置、UI/内容字号作用域隔离、远程配对与撤销')

    # Authentication, account reset and global failures.
    scene('login')
    page.locator('#login-password').fill('wrong')
    page.get_by_role('button', name='登录', exact=True).click()
    expect(page.get_by_role('alert')).to_contain_text('演示口令不正确')
    page.locator('#login-password').fill('demo')
    page.get_by_role('button', name='登录', exact=True).click()
    expect(page.get_by_role('heading', name='桌面总览')).to_be_visible()
    action('page:workbench')
    page.get_by_label('消息', exact=True).fill('账户 A 草稿')
    action('settings:account')
    action('switch-account')
    action('confirm-switch')
    expect(page.get_by_role('heading', name='桌面总览')).to_be_visible()
    action('page:workbench')
    expect(page.get_by_label('消息', exact=True)).to_have_value('')
    expect(page.get_by_text('演示账户 B', exact=True)).to_be_visible()
    scene('startup')
    expect(page.get_by_role('heading', name='无法打开本机数据库')).to_be_visible()
    scene('notfound')
    action('scene:normal')
    passed.append('登录失败恢复、账户草稿隔离、启动错误与404')

    # File opening is a supported path, independent of a build tool or server.
    offline = context.new_page()
    offline.goto(Path(__file__).with_name('index.html').resolve().as_uri())
    expect(offline.get_by_role('heading', name='桌面总览', exact=True)).to_be_visible()
    action('settings:providers', offline)
    expect(offline.get_by_role('dialog')).to_be_visible()
    offline.close()
    passed.append('直接打开本地文件')

    # Layout and focus at desktop narrow width and remote mobile width.
    page.reload()
    page.set_viewport_size({'width': 800, 'height': 900})
    action('page:workbench')
    no_overflow()
    settings()
    page.screenshot(path=str(output / '06-settings-800.png'))
    no_overflow()
    for _ in range(28):
        page.keyboard.press('Tab')
        assert page.evaluate('document.querySelector("#dialog").contains(document.activeElement)'), 'Focus left modal'
    page.keyboard.press('Escape')
    expect(page.get_by_role('dialog')).not_to_be_visible()
    settings('appearance')
    for density in ['1','1.12','1.25']:
        page.locator('#density').select_option(density)
        no_overflow()
    page.screenshot(path=str(output / '07-ui-size-125.png'))
    page.locator('#density').select_option('1')
    action('close-settings')
    action('review')
    action('toggle-theme')
    action('review')
    page.screenshot(path=str(output / '08-workbench-dark.png'))
    page.set_viewport_size({'width': 390, 'height': 844})
    scene('remote')
    no_overflow()
    page.screenshot(path=str(output / '09-remote-mobile.png'))
    passed.append('800px、三档UI尺寸、暗色、远程390px与模态键盘焦点')

    # Exclude the explicitly tested file assets from the no-network assertion.
    external = [u for u in external if not u.startswith('file:')]
    assert not errors, errors
    assert not external, external
    print('\n'.join('PASS ' + name for name in passed))
    print(f'{len(passed)}/{len(passed)} scenario groups passed; 0 page errors; 0 external HTTP requests')
    browser.close()
