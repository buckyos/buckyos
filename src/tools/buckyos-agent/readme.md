# BuckyOS Agent

buckyos-agent tools 是用ts编写的，运行在nodejs环境下的buckycli工具。
会默认打入opendan的docker镜像(paios/aios), 是opendan agent runtime为Agent提供的，访问buckyos的基础工具
是用ts编写是方便开放源代码给Agent,Agent可以在此代码基础上，自行升级

## MVP功能

1. 通过control_panel的接口发布static-web-app
应为jarvis的权限问题，只能执行到store,需要传一个页面url让用户自己点开，确认上线
发布到哪里的问题？
- 一个通用的publish目录，放在里面的所有文件夹立刻就能得到预览
- 打包成app发布，需要用户手工鉴权

2. opendan组件，让opendan的那些内置命令，可以以真正的bash cmd的方式存在
opendan本身的接口也要标准化
4个元工具
session 
todo
workspace


3. 为了拼接URL方便，得到系统的基本信息

4. 操作NDN，打开数据，写入数据