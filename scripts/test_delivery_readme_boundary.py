"""移动端边界文字正反例；不将无关的未测项当作手机未认证声明。"""
import unittest
from dmghg_delivery_oracle import discloses_mobile_verification_boundary as disclosed


class ReadmeBoundaryTests(unittest.TestCase):
    def test_legitimate_chinese_and_english_negative_statements(self):
        examples = [
            'Android独立运行未验证。',
            '本方案未在 Android 原生移动端上独立运行验证，且无法直接脱机移植至手机端。',
            '手机端尚未进行独立实测。',
            '移动端还没有完成运行认证。',
            'Android standalone is not yet independently tested.',
            'Android standalone remains unverified.',
            '手机独立运行待实机验证。',
        ]
        for text in examples:
            with self.subTest(text=text):
                self.assertTrue(disclosed(text))

    def test_verified_mobile_and_unrelated_untested_items_do_not_pass(self):
        examples = [
            'Android已完整验证通过。PC性能未验证。',
            'Android已经可用；长期稳定性未验证。',
            '手机可以独立运行。\n其他频道未测试。',
            'Windows功能未验证。',
            'Android supports standalone playback.',
            '',
        ]
        for text in examples:
            with self.subTest(text=text):
                self.assertFalse(disclosed(text))


if __name__=='__main__':
    unittest.main()
