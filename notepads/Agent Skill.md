# Agent Skill & Tools

没有默认行为！ Agent Skill也是通过提示词文本渲染引擎显示注入到系统提示词中的。

注意Skills的配置和Tools是隔离的。互相独立配置，不会影响。实际上tools用的很少

## 基本状态

__OPENDAN_VAR(session_skill_list,$session_skill_list.$num)：获取最近的num个可用的skill，组成skill_list
__OPENDAN_CONTENT(session_skills)__ : 将“已经加载的skills"，插入到提示词中

## Agent tool 支持
- load_skill <skillname> <behavior|session> 在当前behavior（默认） | session 中加载制定的skill,skill可以是路径
加载skill成功后，会影响__OPENDAN_CONTENT(session_skills)__ 的值，以及__OPENDAN_VAR(session_skill_list的顺序
- unload_skill <skillname> 卸载制定的skill
查找skill用标准的bash文件查找工具就可以了，不需要有额外的cmd支持

## Agent envirmement中的session_skills来源
是下面几个集合的并集

session.load_skills (通常为空)，可以被load_skill $skill_name session 影响
behavior.load_skills ,在beahvior的cfg里定义，可以被load_skill $skill_name 影响
behavior的skills配置里可以有mode设置，说明和session.load_skills的关系（默认是并集）

当beahvior切换时，behavior.load_skills也会切换，最终影响Agent envirmement中的session_skills

## Skills的目录结构
一个skill至少有2个文件,skill.md是本体，meta.json是其原信息，包括会出现在Skill_list中的简介
$agent_root/skills/$skill_name/meta.json
$agent_root/skills/$skill_name/skill.md



