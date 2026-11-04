"""Theme design-asset checks. No production/host acceptance claims."""
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
    page = browser.new_page(viewport={'width': 1440, 'height': 1000})
    errors, external, passed = [], [], []
    page.on('pageerror', lambda error: errors.append(str(error)))
    page.on('request', lambda request: external.append(request.url) if not request.url.startswith(args.url) else None)

    def action(name):
        if name.startswith('reset-') and name != 'reset-scope':
            page.locator('.theme-reset-menu').evaluate('e=>e.open=true')
        page.locator(f'[data-appearance-action="{name}"]').click()

    def open_editor():
        page.locator('[data-action="settings:providers"]').first.click()
        page.locator('[data-action="section:appearance"]').click()

    def css(selector, key):
        return page.locator(selector).first.evaluate('(e,key)=>getComputedStyle(e).getPropertyValue(key).trim()', key)

    def no_overflow():
        assert page.evaluate('document.documentElement.scrollWidth <= innerWidth + 1')
        assert page.locator('.settings-main').evaluate('e=>e.scrollWidth <= e.clientWidth + 1')

    page.goto(args.url)
    page.wait_for_load_state('networkidle')
    page.locator('[data-action="page:workbench"]').first.click()
    open_editor()
    original = css('html', '--blue')
    action('preset:paper')
    assert css('html', '--blue') == '#985B05'
    assert page.locator('[data-appearance-field="fullPreview"]').count() == 0
    assert page.locator('.theme-preview').count() == 0
    page.screenshot(path=str(output / 'theme-light.png'))
    page.keyboard.press('Escape')
    expect(page.get_by_role('dialog')).not_to_be_visible()
    assert css('html', '--blue') == original
    passed.append('整窗实时预览、无缩略预览控件、Esc恢复')

    open_editor()
    page.locator('#color-accent').fill('#126647')
    page.locator('#color-accent').press('Tab')
    action('preset:slate')
    expect(page.locator('#color-accent')).to_have_value('#126647')
    action('inherit:accent')
    expect(page.locator('#color-accent')).to_have_value('#6942B5')
    page.locator('#theme-scope').select_option('reading')
    page.locator('#color-background').fill('#FFF9ED')
    page.locator('#color-background').press('Tab')
    assert css('html', '--reading-background') == '#FFF9ED'
    assert css('html', '--code-background') == '#F5F5FA'
    action('apply')
    action('cancel')
    expect(page.locator('#color-background')).to_have_value('#FFF9ED')
    page.locator('[data-action="close-settings"]').first.click()
    assert css('.assistant-content', 'background-color') == 'rgb(255, 249, 237)'
    passed.append('套装保留覆盖、单项继承、阅读/代码隔离、应用后取消')

    open_editor()
    action('edit:dark')
    action('preset:forest')
    expect(page.locator('html')).to_have_attribute('data-theme', 'light')
    action('mode:auto')
    action('apply')
    page.emulate_media(color_scheme='dark')
    expect(page.locator('html')).to_have_attribute('data-theme', 'dark')
    assert css('html', '--blue') == '#89D8AF'
    page.screenshot(path=str(output / 'theme-dark.png'))
    page.emulate_media(color_scheme='light')
    expect(page.locator('html')).to_have_attribute('data-theme', 'light')
    assert css('html', '--blue') == '#6942B5'
    passed.append('编辑明暗独立、跟随系统、两份套装保留')

    ui_before = page.locator('.session-bar').bounding_box()['height']
    code_before = css('.code-panel pre', 'font-size')
    page.locator('#content-size').select_option('20')
    assert css('#chat-input', 'font-size') == '20px'
    assert css('.code-panel pre', 'font-size') == code_before
    assert page.locator('.session-bar').bounding_box()['height'] == ui_before
    page.locator('#codeSize').select_option('18')
    assert css('.code-panel pre', 'font-size') == '18px'
    page.locator('#density').select_option('1.25')
    assert page.locator('.session-bar').bounding_box()['height'] > ui_before
    assert css('#chat-input', 'font-size') == '20px'
    assert css('.code-panel pre', 'font-size') == '18px'
    page.locator('#readingFont').select_option('serif')
    assert 'Songti' in css('#chat-input', 'font-family')
    assert 'Songti' not in css('.code-panel pre', 'font-family')
    page.locator('[data-action="section:about"]').click()
    assert css('#chat-input', 'font-size') == '13px'
    passed.append('正文/代码/界面密度隔离、字体区域隔离、切分类恢复')

    page.locator('[data-action="section:appearance"]').click()
    action('edit:light')
    page.locator('#theme-scope').select_option('interface')
    page.locator('#color-foreground').fill('#F5F5FA')
    page.locator('#color-foreground').press('Tab')
    expect(page.locator('[data-appearance-action="apply"]')).to_be_disabled()
    expect(page.locator('#theme-validation')).to_contain_text('对比度不足')
    action('inherit:foreground')
    expect(page.locator('[data-appearance-action="apply"]')).to_be_enabled()
    page.locator('#color-accent').fill('invalid')
    page.locator('#color-accent').press('Tab')
    expect(page.locator('[data-appearance-action="apply"]')).to_be_disabled()
    action('reset-scope')
    expect(page.locator('[data-appearance-action="apply"]')).to_be_enabled()
    action('reset-all')
    action('apply')
    assert css('html', '--blue') == original
    passed.append('无效HEX、低对比度阻止应用、恢复区域与全部默认')

    # Every preset passes the implemented basic checks in both modes.
    for mode in ['light', 'dark']:
        action('edit:' + mode)
        for preset in ['studio', 'precision', 'paper', 'forest', 'slate']:
            action('preset:' + preset)
            expect(page.locator('[data-appearance-action="apply"]')).to_be_enabled()
    passed.append('五套主题十份明暗基础配对')

    page.set_viewport_size({'width': 800, 'height': 900})
    page.locator('#uiSize').select_option('16')
    page.locator('#density').select_option('1.25')
    page.locator('#content-size').select_option('20')
    page.locator('#codeSize').select_option('18')
    no_overflow()
    action('apply')
    page.locator('[data-appearance-action="apply"]').focus()
    page.keyboard.press('Tab')
    assert page.evaluate('document.querySelector("#dialog").contains(document.activeElement)')
    page.screenshot(path=str(output / 'theme-800-large.png'))
    page.keyboard.press('Escape')
    open_editor()
    page.locator('details.theme-section').click()
    page.locator('#motion').select_option('reduce')
    action('apply')
    assert css('.theme-preset', 'transition-duration') == '0s'
    passed.append('800px最大字号与密度、键盘焦点、减少动效')

    assert not errors, errors
    assert not external, external
    print('\n'.join('PASS ' + name for name in passed))
    print(f'{len(passed)}/{len(passed)} theme groups passed; 0 page errors; 0 external HTTP requests')
    browser.close()
