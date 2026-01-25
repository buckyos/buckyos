# BuckyOS Control Panel

如其名，BuckyOS Control Panel提供了BuckyOS的系统控制面板，包含UI和一组API。

BuckyOS Control Panel是系统服务，使用短域名 sys （https://sys.$zonehostname,https://sys-$zonehostname)

所有的popup，都允许在sys域下以业内组件的形式存在，保持体验的丝滑

## Dashboard

** /index.html 

## SSO

** /sso/login.html (弹窗)

** /login_index.html


## App安装协议

** /install.html (弹窗)
** /share_app.html

## Publish Content

任何应用都可以拉起该页面，在Zone级别发布一个内容（文件/文件夹）。
内容的发布形式由Zone全局管理. 可以是常规的Share,也可以是真正的，基于cyfs://的publish
发布成功后，可以得到一个发布结果的“收据",通过该收据，应用可以拉起系统详情页查看，也可以用API查询发布状态。

** /ndn/publish.html 

** /my_content.html?content_id=xxxx


## App Store
独立成另一个系统服务，不在本PRD中描述





